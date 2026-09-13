use std::time::Duration;

use reqwest::blocking::{Client, multipart};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use vpr_integration::{
    CancellationProbe, PcmSampleFormat, ProviderDescriptor, ProviderError, ProviderErrorKind,
    SttPort, SttRequest, Transcript, UsageEvidence, UsageUnit,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiTranscriptionConfig {
    endpoint: String,
    api_key: String,
    model: String,
    timeout: Duration,
}

impl OpenAiTranscriptionConfig {
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
            provider: "openai-transcription".to_owned(),
            model: self.model.clone(),
            representation: Some("speech-to-text".to_owned()),
        }
    }
}

pub struct OpenAiTranscriptionStt {
    client: Client,
    config: OpenAiTranscriptionConfig,
}

impl OpenAiTranscriptionStt {
    /// Builds a batch transcription adapter without exposing its credential.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint policy or client construction fails.
    pub fn new(config: OpenAiTranscriptionConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }
}

impl SttPort for OpenAiTranscriptionStt {
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
        let wav = encode_wav(&request.audio)?;
        let part = multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|_| invalid_response())?;
        let mut form = multipart::Form::new()
            .text("model", self.config.model.clone())
            .part("file", part);
        if let Some(language) = request.locale_hint.as_deref().and_then(language_code) {
            form = form.text("language", language.to_owned());
        }
        let response = self
            .client
            .post(&self.config.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .multipart(form)
            .send()
            .map_err(|error| map_transport_error(&error))?;
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if !response.status().is_success() {
            return Err(map_status(response.status().as_u16()));
        }
        let payload: TranscriptionResponse = response.json().map_err(|_| invalid_response())?;
        if payload.text.trim().is_empty() {
            return Err(invalid_response());
        }
        let locale = payload
            .language
            .or_else(|| request.locale_hint.clone())
            .unwrap_or_else(|| "und".to_owned());
        Ok((
            Transcript {
                text: payload.text,
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
struct TranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

fn encode_wav(audio: &vpr_integration::AudioInput) -> Result<Vec<u8>, ProviderError> {
    if audio.sample_format != PcmSampleFormat::S16Le || !audio.is_well_formed() {
        return Err(invalid_response());
    }
    let data_len = u32::try_from(audio.pcm.len()).map_err(|_| invalid_response())?;
    let channels = audio.channels;
    let block_align = channels.checked_mul(2).ok_or_else(invalid_response)?;
    let byte_rate = audio
        .sample_rate_hz
        .checked_mul(u32::from(block_align))
        .ok_or_else(invalid_response)?;
    let riff_size = 36_u32.checked_add(data_len).ok_or_else(invalid_response)?;
    let mut wav = Vec::with_capacity(44 + audio.pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&audio.sample_rate_hz.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&audio.pcm);
    Ok(wav)
}

fn language_code(locale: &str) -> Option<&str> {
    let code = locale.split(['-', '_']).next()?.trim();
    (code.len() >= 2 && code.len() <= 3 && code.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .then_some(code)
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
