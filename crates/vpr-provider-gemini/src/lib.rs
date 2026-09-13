use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, ProviderDescriptor, ProviderError,
    ProviderErrorKind, UsageEvidence,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct GeminiConfig {
    endpoint: String,
    api_key: String,
    model: String,
    timeout: Duration,
}

impl GeminiConfig {
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
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "gemini".to_owned(),
            model: self.model.clone(),
            representation: None,
        }
    }
}

pub struct GeminiLlm {
    client: Client,
    config: GeminiConfig,
}

impl GeminiLlm {
    /// Builds a Gemini Interactions streaming adapter without exposing credentials.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint policy or client construction fails.
    pub fn new(config: GeminiConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }

    fn execute(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let body = InteractionRequest {
            model: &self.config.model,
            input: &request.context,
            stream: true,
        };
        let response = self
            .client
            .post(&self.config.endpoint)
            .header("x-goog-api-key", &self.config.api_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|error| map_transport_error(&error))?;
        consume_response(response, cancellation, sink)
    }
}
impl LlmPort for GeminiLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
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

#[derive(Serialize)]
struct InteractionRequest<'a> {
    model: &'a str,
    input: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GeminiEvent {
    event_type: String,
    #[serde(default)]
    delta: Option<GeminiDelta>,
    #[serde(default)]
    interaction: Option<GeminiInteraction>,
}
#[derive(Deserialize, Default)]
struct GeminiDelta {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct GeminiInteraction {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<GeminiUsage>,
}

#[derive(Deserialize, Default)]
struct GeminiUsage {
    #[serde(default)]
    total_input_tokens: Option<u64>,
    #[serde(default)]
    total_output_tokens: Option<u64>,
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
    let mut saw_completed = false;
    let mut saw_done = false;
    for line in BufReader::new(response).lines() {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let line = line.map_err(|_| invalid_response())?;
        let Some(payload) = line.strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        if payload == "[DONE]" {
            saw_done = true;
            continue;
        }
        let event: GeminiEvent = serde_json::from_str(payload).map_err(|_| invalid_response())?;
        apply_event(event, sink, &mut usage, &mut saw_completed)?;
    }
    if !saw_completed || !saw_done {
        return Err(invalid_response());
    }
    Ok(usage)
}

fn apply_event(
    event: GeminiEvent,
    sink: &mut dyn GeneratedTextSink,
    usage: &mut UsageEvidence,
    saw_completed: &mut bool,
) -> Result<(), ProviderError> {
    match event.event_type.as_str() {
        "step.delta" => {
            let delta = event.delta.ok_or_else(invalid_response)?;
            if delta.delta_type == "text"
                && let Some(text) = delta.text
                && !text.is_empty()
            {
                sink.push_generated_text(&text)?;
            }
        }
        "interaction.completed" => {
            let interaction = event.interaction.ok_or_else(invalid_response)?;
            if interaction.status.as_deref() != Some("completed") {
                return Err(invalid_response());
            }
            if let Some(event_usage) = interaction.usage {
                usage.input_units = event_usage.total_input_tokens;
                usage.output_units = event_usage.total_output_tokens;
            }
            *saw_completed = true;
        }
        "error" => {
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: true,
            });
        }
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
