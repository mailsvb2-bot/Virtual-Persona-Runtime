use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, LlmTextStream, ProviderDescriptor,
    ProviderError, ProviderErrorKind, UsageEvidence, UsageUnit,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiCompatibleConfig {
    provider_name: String,
    endpoint: String,
    api_key: String,
    model: String,
    timeout: Duration,
    max_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    thinking: Option<ThinkingConfig>,
}

impl OpenAiCompatibleConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            provider_name: "openai-compatible".to_owned(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
            timeout: DEFAULT_TIMEOUT,
            max_tokens: None,
            reasoning_effort: None,
            thinking: None,
        }
    }

    #[must_use]
    pub fn with_provider_name(mut self, provider_name: impl Into<String>) -> Self {
        let provider_name = provider_name.into();
        if !provider_name.trim().is_empty() {
            self.provider_name = provider_name;
        }
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        if max_tokens > 0 {
            self.max_tokens = Some(max_tokens);
        }
        self
    }

    #[must_use]
    pub fn with_reasoning_effort(mut self, reasoning_effort: impl Into<String>) -> Self {
        let reasoning_effort = reasoning_effort.into();
        if !reasoning_effort.trim().is_empty() {
            self.reasoning_effort = Some(reasoning_effort);
        }
        self
    }

    #[must_use]
    pub const fn with_thinking_disabled(mut self) -> Self {
        self.thinking = Some(ThinkingConfig { r#type: "disabled" });
        self
    }

    #[must_use]
    pub fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: self.provider_name.clone(),
            model: self.model.clone(),
            representation: None,
        }
    }
}

pub struct OpenAiCompatibleLlm {
    client: Client,
    config: OpenAiCompatibleConfig,
}

impl OpenAiCompatibleLlm {
    /// Builds an HTTPS-capable adapter without exposing the API key through Debug/accessors.
    ///
    /// # Errors
    /// Returns a typed provider configuration error if the HTTP client cannot be built.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }

    fn start_response(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Response, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let mut messages = Vec::with_capacity(2);
        if let Some(instructions) = request.instructions.as_deref() {
            messages.push(ChatMessage {
                role: "system",
                content: instructions,
            });
        }
        messages.push(ChatMessage {
            role: "user",
            content: &request.user_input,
        });
        let body = ChatRequest {
            model: &self.config.model,
            messages,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
            max_tokens: self.config.max_tokens,
            reasoning_effort: self.config.reasoning_effort.as_deref(),
            thinking: self.config.thinking,
        };
        self.client
            .post(&self.config.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|error| map_transport_error(&error))
    }

    fn execute(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        let response = self.start_response(request, cancellation)?;
        Self::consume_response(response, cancellation, sink)
    }

    fn consume_response(
        response: Response,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        let mut stream = OpenAiTextStream::new(response)?;
        while let Some(chunk) = stream.next_chunk(cancellation)? {
            sink.push_generated_text(&chunk)?;
        }
        Ok(stream.usage())
    }
}

impl LlmPort for OpenAiCompatibleLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
    }

    fn open_stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn LlmTextStream>, ProviderError> {
        let response = self.start_response(request, cancellation)?;
        Ok(Box::new(OpenAiTextStream::new(response)?))
    }

    fn stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.execute(request, cancellation, sink)
    }
}

struct OpenAiTextStream {
    lines: std::io::Lines<BufReader<Response>>,
    usage: UsageEvidence,
    completed: bool,
}

impl OpenAiTextStream {
    fn new(response: Response) -> Result<Self, ProviderError> {
        if !response.status().is_success() {
            return Err(map_status(response.status().as_u16()));
        }
        Ok(Self {
            lines: BufReader::new(response).lines(),
            usage: UsageEvidence::default(),
            completed: false,
        })
    }
}

impl LlmTextStream for OpenAiTextStream {
    fn next_chunk(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<String>, ProviderError> {
        if self.completed {
            return Ok(None);
        }
        loop {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let Some(line) = self.lines.next() else {
                return Err(invalid_response());
            };
            let line = line.map_err(|_| invalid_response())?;
            let Some(payload) = line.strip_prefix("data:").map(str::trim_start) else {
                continue;
            };
            if payload == "[DONE]" {
                self.completed = true;
                return Ok(None);
            }
            let event: ChatChunk = serde_json::from_str(payload).map_err(|_| invalid_response())?;
            if let Some(event_usage) = event.usage {
                self.usage.input_units = event_usage.prompt_tokens;
                self.usage.input_unit = event_usage.prompt_tokens.map(|_| UsageUnit::Token);
                self.usage.output_units = event_usage.completion_tokens;
                self.usage.output_unit = event_usage.completion_tokens.map(|_| UsageUnit::Token);
            }
            let text: String = event
                .choices
                .into_iter()
                .filter_map(|choice| choice.delta.content)
                .filter(|content| !content.is_empty())
                .collect();
            if !text.is_empty() {
                return Ok(Some(text));
            }
        }
    }

    fn usage(&self) -> UsageEvidence {
        self.usage.clone()
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ThinkingConfig {
    r#type: &'static str,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

fn validate_endpoint(endpoint: &str) -> Result<(), ProviderError> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| invalid_response())?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(ProviderError {
            kind: ProviderErrorKind::PolicyDenied,
            retryable: false,
        });
    }
    Ok(())
}

fn map_transport_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        return ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        };
    }
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: true,
    }
}

fn map_status(status: u16) -> ProviderError {
    match status {
        401 | 403 => ProviderError {
            kind: ProviderErrorKind::PolicyDenied,
            retryable: false,
        },
        408 => ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        },
        429 => ProviderError {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        },
        500..=599 => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        _ => invalid_response(),
    }
}

fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

#[cfg(test)]
mod tests;
