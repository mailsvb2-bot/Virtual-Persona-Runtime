use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind, SttPort, SttRequest,
    Transcript, UsageEvidence, UsageUnit,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DeepgramSttConfig {
    endpoint: String,
    api_key: String,
    model: String,
    smart_format: bool,
    timeout: Duration,
}

impl DeepgramSttConfig {
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
            smart_format: true,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_smart_format(mut self, smart_format: bool) -> Self {
        self.smart_format = smart_format;
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
            provider: "deepgram".to_owned(),
            model: self.model.clone(),
            representation: Some("speech-to-text".to_owned()),
        }
    }
}

pub struct DeepgramStt {
    client: Client,
    config: DeepgramSttConfig,
}

impl DeepgramStt {
    /// Builds a Deepgram pre-recorded STT adapter without exposing its credential.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint policy or client construction fails.
    pub fn new(config: DeepgramSttConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }
}

impl SttPort for DeepgramStt {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
    }

    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if !request.audio.is_well_formed() {
            return Err(invalid_response());
        }
        let duration_millis = request
            .audio
            .duration_millis()
            .ok_or_else(invalid_response)?;
        let mut url = reqwest::Url::parse(&self.config.endpoint).map_err(|_| invalid_response())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("model", &self.config.model);
            query.append_pair("encoding", "linear16");
            query.append_pair("sample_rate", &request.audio.sample_rate_hz.to_string());
            query.append_pair("channels", &request.audio.channels.to_string());
            query.append_pair(
                "smart_format",
                if self.config.smart_format {
                    "true"
                } else {
                    "false"
                },
            );
            if let Some(locale) = request
                .locale_hint
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(normalize_deepgram_language)
            {
                query.append_pair("language", locale);
            }
        }
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, format!("Token {}", self.config.api_key))
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(request.audio.pcm.clone())
            .send()
            .map_err(|error| map_transport_error(&error))?;
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if !response.status().is_success() {
            return Err(map_status(response.status().as_u16()));
        }
        let payload: DeepgramResponse = response.json().map_err(|_| invalid_response())?;
        let alternative = payload
            .results
            .channels
            .first()
            .and_then(|channel| channel.alternatives.first())
            .ok_or_else(invalid_response)?;
        if alternative.transcript.trim().is_empty() {
            return Err(invalid_response());
        }
        let locale = alternative
            .languages
            .first()
            .cloned()
            .or_else(|| request.locale_hint.clone())
            .unwrap_or_else(|| "und".to_owned());
        Ok((
            Transcript {
                text: alternative.transcript.clone(),
                locale,
            },
            UsageEvidence {
                input_units: Some(duration_millis),
                input_unit: Some(UsageUnit::AudioMillisecond),
                ..UsageEvidence::default()
            },
        ))
    }
}

#[derive(Deserialize)]
struct DeepgramResponse {
    results: DeepgramResults,
}

#[derive(Deserialize)]
struct DeepgramResults {
    channels: Vec<DeepgramChannel>,
}

#[derive(Deserialize)]
struct DeepgramChannel {
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    #[serde(default)]
    languages: Vec<String>,
}

fn normalize_deepgram_language(locale: &str) -> &str {
    if locale.eq_ignore_ascii_case("ru-RU") {
        "ru"
    } else {
        locale
    }
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
