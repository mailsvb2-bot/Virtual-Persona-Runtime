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
const DEFAULT_MAX_TOKENS: u32 = 1024;
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicConfig {
    endpoint: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    timeout: Duration,
}

impl AnthropicConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "anthropic".to_owned(),
            model: self.model.clone(),
            representation: None,
        }
    }
}
pub struct AnthropicLlm {
    client: Client,
    config: AnthropicConfig,
}

impl AnthropicLlm {
    /// Builds a Claude Messages adapter without exposing credentials through public accessors.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint policy or client construction fails.
    pub fn new(config: AnthropicConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        if config.max_tokens == 0 {
            return Err(invalid_response());
        }
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
        let body = MessageRequest {
            model: &self.config.model,
            max_tokens: self.config.max_tokens,
            messages: [MessageInput {
                role: "user",
                content: &request.context,
            }],
            stream: true,
        };
        self.client
            .post(&self.config.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .header("anthropic-version", API_VERSION)
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
        consume_response(response, cancellation, sink)
    }
}

impl LlmPort for AnthropicLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
    }

    fn open_stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn LlmTextStream>, ProviderError> {
        let response = self.start_response(request, cancellation)?;
        Ok(Box::new(AnthropicTextStream::new(response)?))
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

struct AnthropicTextStream {
    lines: std::io::Lines<BufReader<Response>>,
    usage: UsageEvidence,
    completed: bool,
}

impl AnthropicTextStream {
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

impl LlmTextStream for AnthropicTextStream {
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
            let event: StreamEvent =
                serde_json::from_str(payload).map_err(|_| invalid_response())?;
            match event.event_type.as_str() {
                "message_start" => {
                    let message = event.message.ok_or_else(invalid_response)?;
                    self.usage.input_units = message.usage.input_tokens;
                    self.usage.input_unit = message.usage.input_tokens.map(|_| UsageUnit::Token);
                    self.usage.output_units = message.usage.output_tokens;
                    self.usage.output_unit = message.usage.output_tokens.map(|_| UsageUnit::Token);
                }
                "content_block_delta" => {
                    let delta = event.delta.ok_or_else(invalid_response)?;
                    if delta.delta_type == "text_delta"
                        && let Some(text) = delta.text
                        && !text.is_empty()
                    {
                        return Ok(Some(text));
                    }
                }
                "message_delta" => {
                    if let Some(event_usage) = event.usage {
                        self.usage.output_units = event_usage.output_tokens;
                        self.usage.output_unit =
                            event_usage.output_tokens.map(|_| UsageUnit::Token);
                    }
                }
                "message_stop" => {
                    self.completed = true;
                    return Ok(None);
                }
                "error" => return Err(map_stream_error(event.error.as_ref())),
                _ => {}
            }
        }
    }

    fn usage(&self) -> UsageEvidence {
        self.usage.clone()
    }
}

#[derive(Serialize)]
struct MessageRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: [MessageInput<'a>; 1],
    stream: bool,
}

#[derive(Serialize)]
struct MessageInput<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    message: Option<MessageStart>,
    #[serde(default)]
    delta: Option<EventDelta>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    error: Option<AnthropicErrorBody>,
}

#[derive(Deserialize)]
struct MessageStart {
    usage: AnthropicUsage,
}

#[derive(Deserialize, Default)]
struct EventDelta {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct AnthropicErrorBody {
    #[serde(rename = "type")]
    error_type: String,
}
fn consume_response(
    response: Response,
    cancellation: &dyn CancellationProbe,
    sink: &mut dyn GeneratedTextSink,
) -> Result<UsageEvidence, ProviderError> {
    if !response.status().is_success() {
        return Err(map_status(response.status().as_u16()));
    }
    let mut usage = UsageEvidence::default();
    let mut saw_stop = false;
    for line in BufReader::new(response).lines() {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let line = line.map_err(|_| invalid_response())?;
        let Some(payload) = line.strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        let event: StreamEvent = serde_json::from_str(payload).map_err(|_| invalid_response())?;
        apply_event(event, sink, &mut usage, &mut saw_stop)?;
    }
    if !saw_stop {
        return Err(invalid_response());
    }
    Ok(usage)
}

fn apply_event(
    event: StreamEvent,
    sink: &mut dyn GeneratedTextSink,
    usage: &mut UsageEvidence,
    saw_stop: &mut bool,
) -> Result<(), ProviderError> {
    match event.event_type.as_str() {
        "message_start" => {
            let message = event.message.ok_or_else(invalid_response)?;
            usage.input_units = message.usage.input_tokens;
            usage.input_unit = message.usage.input_tokens.map(|_| UsageUnit::Token);
            usage.output_units = message.usage.output_tokens;
            usage.output_unit = message.usage.output_tokens.map(|_| UsageUnit::Token);
        }
        "content_block_delta" => {
            let delta = event.delta.ok_or_else(invalid_response)?;
            if delta.delta_type == "text_delta"
                && let Some(text) = delta.text
                && !text.is_empty()
            {
                sink.push_generated_text(&text)?;
            }
        }
        "message_delta" => {
            if let Some(event_usage) = event.usage {
                usage.output_units = event_usage.output_tokens;
                usage.output_unit = event_usage.output_tokens.map(|_| UsageUnit::Token);
            }
        }
        "message_stop" => *saw_stop = true,
        "error" => return Err(map_stream_error(event.error.as_ref())),
        _ => {}
    }
    Ok(())
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

fn map_stream_error(error: Option<&AnthropicErrorBody>) -> ProviderError {
    match error.map(|value| value.error_type.as_str()) {
        Some("rate_limit_error") => ProviderError {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        },
        Some("overloaded_error") => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        Some("authentication_error" | "permission_error") => ProviderError {
            kind: ProviderErrorKind::PolicyDenied,
            retryable: false,
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
