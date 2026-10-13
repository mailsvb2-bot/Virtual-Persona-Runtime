use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use vpr_integration::{
    CancellationProbe, GeneratedAudioSink, MAX_PROVIDER_BINARY_BODY_BYTES, PcmSampleFormat,
    ProviderDescriptor, ProviderError, ProviderErrorKind, TtsPort, TtsRequest, UsageEvidence,
    UsageUnit, build_provider_http_client, read_bounded_provider_stream,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SAMPLE_RATE_HZ: u32 = 24_000;
const SUPPORTED_PCM_RATES: [u32; 4] = [16_000, 22_050, 24_000, 44_100];

pub struct ElevenLabsTtsConfig {
    endpoint: String,
    api_key: String,
    model: String,
    voice_id: String,
    output_sample_rate_hz: u32,
    timeout: Duration,
}

impl ElevenLabsTtsConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        voice_id: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
            voice_id: voice_id.into(),
            output_sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_output_sample_rate_hz(mut self, sample_rate_hz: u32) -> Self {
        self.output_sample_rate_hz = sample_rate_hz;
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
            provider: "elevenlabs".to_owned(),
            model: self.model.clone(),
            representation: Some(self.voice_id.clone()),
        }
    }
}

pub struct ElevenLabsTts {
    client: Client,
    config: ElevenLabsTtsConfig,
}

impl ElevenLabsTts {
    /// Builds an `ElevenLabs` streaming TTS adapter without exposing its credential.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint, PCM format or client construction fails.
    pub fn new(config: ElevenLabsTtsConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        if config.model.trim().is_empty()
            || config.voice_id.trim().is_empty()
            || !SUPPORTED_PCM_RATES.contains(&config.output_sample_rate_hz)
        {
            return Err(invalid_response());
        }
        let client = build_provider_http_client(config.timeout)
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }

    fn request_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = reqwest::Url::parse(&self.config.endpoint).map_err(|_| invalid_response())?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .pop_if_empty()
            .push(&self.config.voice_id)
            .push("stream");
        url.query_pairs_mut().append_pair(
            "output_format",
            &format!("pcm_{}", self.config.output_sample_rate_hz),
        );
        Ok(url)
    }
}

impl TtsPort for ElevenLabsTts {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
    }

    fn synthesize(
        &self,
        request: &TtsRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if request.text.trim().is_empty() {
            return Err(invalid_response());
        }
        let language_code = request.locale_hint.as_deref().and_then(language_code);
        let body = SpeechRequest {
            text: &request.text,
            model_id: &self.config.model,
            language_code,
        };
        let response = self
            .client
            .post(self.request_url()?)
            .header("xi-api-key", &self.config.api_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|error| map_transport_error(&error))?;
        if !response.status().is_success() {
            return Err(map_status(response.status().as_u16()));
        }
        // Deliver frame-aligned PCM as the provider produces it. This is a
        // generation sink, not a browser/private network publisher: runtime
        // authorization still gates avatar egress after synthesis.
        let generated_bytes = stream_pcm(response, cancellation, sink, self.config.output_sample_rate_hz)?;
        let input_chars = u64::try_from(request.text.chars().count()).unwrap_or(u64::MAX);
        let output_millis = u64::try_from(generated_bytes / 2)
            .ok()
            .and_then(|frames| frames.checked_mul(1_000))
            .and_then(|millis| millis.checked_div(u64::from(self.config.output_sample_rate_hz)))
            .ok_or_else(invalid_response)?;
        Ok(UsageEvidence {
            input_units: Some(input_chars),
            input_unit: Some(UsageUnit::TextCharacter),
            output_units: Some(output_millis),
            output_unit: Some(UsageUnit::AudioMillisecond),
            ..UsageEvidence::default()
        })
    }
}

#[derive(Serialize)]
struct SpeechRequest<'a> {
    text: &'a str,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language_code: Option<&'a str>,
}

/// Incremental provider PCM decoding. Frame alignment is preserved across
/// arbitrary HTTP read boundaries (including one-byte chunks). Neither a
/// declared Content-Length nor a chunked transfer can bypass the hard limit.
fn stream_pcm(
    mut response: Response,
    cancellation: &dyn CancellationProbe,
    sink: &mut dyn GeneratedAudioSink,
    sample_rate_hz: u32,
) -> Result<usize, ProviderError> {
    let mut pending = None::<u8>;
    let total = read_bounded_provider_stream(
        response,
        MAX_PROVIDER_BINARY_BODY_BYTES,
        Some(cancellation),
        |bytes| {
            let mut start = 0_usize;
            if let Some(first) = pending.take() {
                let pair = [first, bytes[0]];
                sink.push_generated_audio(&pair, sample_rate_hz, 1, PcmSampleFormat::S16Le)?;
                start = 1;
            }
            let aligned_end = start + ((bytes.len() - start) / 2) * 2;
            if aligned_end > start {
                sink.push_generated_audio(
                    &bytes[start..aligned_end],
                    sample_rate_hz,
                    1,
                    PcmSampleFormat::S16Le,
                )?;
            }
            if aligned_end < bytes.len() {
                pending = Some(bytes[aligned_end]);
            }
            Ok(())
        },
    )?;
    if total == 0 || pending.is_some() {
        return Err(invalid_response());
    }
    Ok(total)
}

fn language_code(locale: &str) -> Option<&str> {
    let code = locale.split(['-', '_']).next()?.trim();
    (code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(code)
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
        ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        }
    } else {
        ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        }
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
