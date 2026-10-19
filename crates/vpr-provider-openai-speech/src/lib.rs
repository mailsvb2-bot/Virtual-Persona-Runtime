use std::io::Read;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use vpr_integration::{
    AudioInput, CancellationProbe, GeneratedAudioSink, MAX_PROVIDER_BINARY_BODY_BYTES,
    PcmSampleFormat, ProviderDescriptor, ProviderError, ProviderErrorKind, TtsPort, TtsRequest,
    UsageEvidence, UsageUnit, build_provider_http_client, read_bounded_provider_body,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiSpeechConfig {
    endpoint: String,
    api_key: String,
    provider_name: String,
    model: String,
    voice: String,
    timeout: Duration,
    stream_pcm: bool,
}

impl OpenAiSpeechConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        voice: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            provider_name: "openai-speech".to_owned(),
            model: model.into(),
            voice: voice.into(),
            timeout: DEFAULT_TIMEOUT,
            stream_pcm: false,
        }
    }

    /// Preserve the actual selected provider identity over a compatible HTTP wire protocol.
    #[must_use]
    pub fn with_provider_name(mut self, name: impl Into<String>) -> Self {
        self.provider_name = name.into();
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Enable raw PCM streaming only for the trusted loopback Piper bridge.
    #[must_use]
    pub const fn with_stream_pcm(mut self, enabled: bool) -> Self {
        self.stream_pcm = enabled;
        self
    }

    #[must_use]
    pub fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: self.provider_name.clone(),
            model: self.model.clone(),
            representation: Some(self.voice.clone()),
        }
    }
}

pub struct OpenAiSpeechTts {
    client: Client,
    config: OpenAiSpeechConfig,
}

impl OpenAiSpeechTts {
    /// Builds an `OpenAI` Speech adapter without exposing its credential.
    ///
    /// # Errors
    /// Returns a typed provider error when endpoint policy or client construction fails.
    pub fn new(config: OpenAiSpeechConfig) -> Result<Self, ProviderError> {
        validate_endpoint(&config.endpoint)?;
        if config.stream_pcm
            && (config.provider_name != "piper" || !is_loopback_endpoint(&config.endpoint))
        {
            return Err(invalid_response());
        }
        if config.provider_name.trim().is_empty()
            || config.model.trim().is_empty()
            || config.voice.trim().is_empty()
        {
            return Err(invalid_response());
        }
        let client = build_provider_http_client(config.timeout)
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self { client, config })
    }
}

impl TtsPort for OpenAiSpeechTts {
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
        let body = SpeechRequest {
            model: &self.config.model,
            input: &request.text,
            voice: &self.config.voice,
            response_format: if self.config.stream_pcm { "pcm" } else { "wav" },
        };
        let response = self
            .client
            .post(&self.config.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|error| {
                if self.config.stream_pcm {
                    eprintln!(
                        "VPR Piper stage=http_send failed timeout={}",
                        error.is_timeout()
                    );
                }
                map_transport_error(&error)
            })?;
        if !response.status().is_success() {
            if self.config.stream_pcm {
                eprintln!(
                    "VPR Piper stage=http_status failed status={}",
                    response.status().as_u16()
                );
            }
            return Err(map_status(response.status().as_u16()));
        }
        if self.config.stream_pcm {
            return stream_pcm_body(response, cancellation, sink, request.text.chars().count());
        }
        let wav = read_body(response, cancellation)?;
        let audio = decode_pcm_wav(&wav)?;
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        sink.push_generated_audio(
            &audio.pcm,
            audio.sample_rate_hz,
            audio.channels,
            audio.sample_format,
        )?;
        let input_chars = u64::try_from(request.text.chars().count()).unwrap_or(u64::MAX);
        let output_millis = audio.duration_millis().ok_or_else(invalid_response)?;
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
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
}

fn read_body(
    response: Response,
    cancellation: &dyn CancellationProbe,
) -> Result<Vec<u8>, ProviderError> {
    let output =
        read_bounded_provider_body(response, MAX_PROVIDER_BINARY_BODY_BYTES, Some(cancellation))?;
    if output.is_empty() {
        return Err(invalid_response());
    }
    Ok(output)
}

fn stream_pcm_body(
    mut response: Response,
    cancellation: &dyn CancellationProbe,
    sink: &mut dyn GeneratedAudioSink,
    text_chars: usize,
) -> Result<UsageEvidence, ProviderError> {
    let sample_rate_hz = response
        .headers()
        .get("x-vpr-sample-rate")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|rate| (8_000..=48_000).contains(rate))
        .ok_or_else(|| {
            eprintln!("VPR Piper stage=pcm_headers failed reason=sample_rate");
            invalid_response()
        })?;
    if response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        != Some("application/octet-stream")
    {
        eprintln!("VPR Piper stage=pcm_headers failed reason=content_type");
        return Err(invalid_response());
    }
    let mut buffer = [0_u8; 8192];
    let mut total = 0_usize;
    let mut pending_byte = None;
    loop {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let size = response.read(&mut buffer).map_err(|error| {
            eprintln!("VPR Piper stage=pcm_read failed kind={:?}", error.kind());
            invalid_response()
        })?;
        if size == 0 {
            break;
        }
        total = total
            .checked_add(size)
            .filter(|count| *count <= MAX_PROVIDER_BINARY_BODY_BYTES)
            .ok_or_else(invalid_response)?;
        let mut chunk = Vec::with_capacity(size + 1);
        if let Some(byte) = pending_byte.take() {
            chunk.push(byte);
        }
        chunk.extend_from_slice(&buffer[..size]);
        if chunk.len() % 2 != 0 {
            pending_byte = chunk.pop();
        }
        if !chunk.is_empty() {
            sink.push_generated_audio(&chunk, sample_rate_hz, 1, PcmSampleFormat::S16Le)
                .inspect_err(|error| {
                    eprintln!("VPR Piper stage=pcm_sink failed kind={:?}", error.kind);
                })?;
        }
    }
    if total == 0 || pending_byte.is_some() {
        eprintln!("VPR Piper stage=pcm_end failed reason=empty_or_partial");
        return Err(invalid_response());
    }
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    let duration = u64::try_from(total)
        .unwrap_or(u64::MAX)
        .saturating_mul(1000)
        / (u64::from(sample_rate_hz) * 2);
    Ok(UsageEvidence {
        input_units: Some(u64::try_from(text_chars).unwrap_or(u64::MAX)),
        input_unit: Some(UsageUnit::TextCharacter),
        output_units: Some(duration),
        output_unit: Some(UsageUnit::AudioMillisecond),
        ..UsageEvidence::default()
    })
}

fn is_loopback_endpoint(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint).ok().is_some_and(|url| {
        url.scheme() == "http"
            && url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
    })
}

fn decode_pcm_wav(bytes: &[u8]) -> Result<AudioInput, ProviderError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid_response());
    }
    let mut offset = 12_usize;
    let mut format = None;
    let mut pcm = None;
    while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[offset..offset + 4];
        let chunk_size = usize::try_from(u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| invalid_response())?,
        ))
        .map_err(|_| invalid_response())?;
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(chunk_size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(invalid_response)?;
        if id == b"fmt " {
            if chunk_size < 16 {
                return Err(invalid_response());
            }
            let audio_format = u16::from_le_bytes(
                bytes[data_start..data_start + 2]
                    .try_into()
                    .map_err(|_| invalid_response())?,
            );
            let channels = u16::from_le_bytes(
                bytes[data_start + 2..data_start + 4]
                    .try_into()
                    .map_err(|_| invalid_response())?,
            );
            let sample_rate = u32::from_le_bytes(
                bytes[data_start + 4..data_start + 8]
                    .try_into()
                    .map_err(|_| invalid_response())?,
            );
            let bits_per_sample = u16::from_le_bytes(
                bytes[data_start + 14..data_start + 16]
                    .try_into()
                    .map_err(|_| invalid_response())?,
            );
            if audio_format != 1 || channels == 0 || sample_rate == 0 || bits_per_sample != 16 {
                return Err(invalid_response());
            }
            format = Some((sample_rate, channels));
        } else if id == b"data" {
            pcm = Some(bytes[data_start..data_end].to_vec());
        }
        offset = data_end
            .checked_add(chunk_size % 2)
            .ok_or_else(invalid_response)?;
    }
    let (sample_rate_hz, channels) = format.ok_or_else(invalid_response)?;
    let audio = AudioInput {
        pcm: pcm.ok_or_else(invalid_response)?,
        sample_rate_hz,
        channels,
        sample_format: PcmSampleFormat::S16Le,
    };
    if !audio.is_well_formed() {
        return Err(invalid_response());
    }
    Ok(audio)
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
