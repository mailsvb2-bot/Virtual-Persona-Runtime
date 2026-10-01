mod avatar;
mod provider_http;
mod transport;

pub use provider_http::{
    BoundedProviderLineReader, MAX_PROVIDER_BINARY_BODY_BYTES, MAX_PROVIDER_JSON_BODY_BYTES,
    MAX_PROVIDER_STREAM_LINE_BYTES, build_provider_http_client, read_bounded_provider_body,
};

pub use avatar::{
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarClientCommand,
    RealtimeAvatarClientControl, RealtimeAvatarClientEvent, RealtimeAvatarClientRoute,
    RealtimeAvatarPort, RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate,
    WebRtcIceServer, WebRtcSessionDescription,
};
pub use transport::{
    MediaTimelineStamp, RealtimeAudioOutputEvent, RealtimeMediaFlushEvent, RealtimeOutputPort,
    RealtimeTextOutputEvent, RealtimeVideoOutputEvent, TransportError, TransportErrorKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider: String,
    pub model: String,
    pub representation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Unavailable,
    RateLimited,
    Timeout,
    Cancelled,
    InvalidResponse,
    PolicyDenied,
    InsufficientCredits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageUnit {
    Token,
    TextCharacter,
    AudioMillisecond,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageEvidence {
    pub input_units: Option<u64>,
    pub input_unit: Option<UsageUnit>,
    pub output_units: Option<u64>,
    pub output_unit: Option<UsageUnit>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

pub trait CancellationProbe: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

mod llm;
mod stt;

pub use llm::{
    GeneratedTextBuffer, GeneratedTextSink, LlmPort, LlmRequest, LlmTextStream,
    MAX_GENERATED_TEXT_BYTES, TimedGeneratedTextBuffer,
};
pub use stt::{SttAudioStream, SttPort, SttRequest, SttStreamEvent, SttStreamRequest, Transcript};

mod sealed {
    pub trait GeneratedAudioSink {}
    pub trait GeneratedVideoSink {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmSampleFormat {
    S16Le,
}

/// Maximum retained PCM bytes for one provider-generated audio result.
pub const MAX_GENERATED_AUDIO_BYTES: usize = 16 * 1024 * 1024;
/// Maximum retained duration for one provider-generated audio result.
pub const MAX_GENERATED_AUDIO_DURATION_MILLIS: u64 = 300_000;
/// Maximum retained encoded bytes for one provider-generated video result.
pub const MAX_GENERATED_VIDEO_BYTES: usize = 32 * 1024 * 1024;
/// Maximum retained frames for one provider-generated video result.
pub const MAX_GENERATED_VIDEO_FRAMES: usize = 9_000;
/// Maximum timestamp span retained for one provider-generated video result.
pub const MAX_GENERATED_VIDEO_DURATION_MICROS: u64 = 300_000_000;

impl PcmSampleFormat {
    #[must_use]
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::S16Le => 2,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AudioInput {
    pub pcm: Vec<u8>,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: PcmSampleFormat,
}

impl std::fmt::Debug for AudioInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AudioInput")
            .field("pcm_bytes", &self.pcm.len())
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("channels", &self.channels)
            .field("sample_format", &self.sample_format)
            .finish()
    }
}

impl AudioInput {
    #[must_use]
    pub const fn bytes_per_sample(&self) -> usize {
        self.sample_format.bytes_per_sample()
    }

    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        let frame_bytes = usize::from(self.channels).checked_mul(self.bytes_per_sample());
        self.sample_rate_hz > 0
            && self.channels > 0
            && !self.pcm.is_empty()
            && frame_bytes.is_some_and(|size| size > 0 && self.pcm.len() % size == 0)
    }

    #[must_use]
    pub fn duration_millis(&self) -> Option<u64> {
        if !self.is_well_formed() {
            return None;
        }
        let frame_bytes =
            u64::try_from(usize::from(self.channels) * self.bytes_per_sample()).ok()?;
        let pcm_len = u64::try_from(self.pcm.len()).ok()?;
        let frames = pcm_len / frame_bytes;
        frames
            .checked_mul(1_000)?
            .checked_div(u64::from(self.sample_rate_hz))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRequest {
    pub text: String,
    pub locale_hint: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct GeneratedAudioBuffer {
    pcm: Vec<u8>,
    sample_rate_hz: Option<u32>,
    channels: Option<u16>,
    sample_format: Option<PcmSampleFormat>,
}

impl std::fmt::Debug for GeneratedAudioBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedAudioBuffer")
            .field("pcm_bytes", &self.pcm.len())
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("channels", &self.channels)
            .field("sample_format", &self.sample_format)
            .finish()
    }
}

impl GeneratedAudioBuffer {
    #[must_use]
    pub fn pcm(&self) -> &[u8] {
        &self.pcm
    }

    #[must_use]
    pub const fn sample_rate_hz(&self) -> Option<u32> {
        self.sample_rate_hz
    }

    #[must_use]
    pub const fn channels(&self) -> Option<u16> {
        self.channels
    }

    #[must_use]
    pub const fn sample_format(&self) -> Option<PcmSampleFormat> {
        self.sample_format
    }

    #[must_use]
    pub fn duration_millis(&self) -> Option<u64> {
        let sample_rate_hz = self.sample_rate_hz?;
        let channels = self.channels?;
        let sample_format = self.sample_format?;
        if self.pcm.is_empty() || sample_rate_hz == 0 || channels == 0 {
            return None;
        }
        let bytes_per_sample = match sample_format {
            PcmSampleFormat::S16Le => 2_u64,
        };
        let frame_bytes = u64::from(channels).checked_mul(bytes_per_sample)?;
        let pcm_len = u64::try_from(self.pcm.len()).ok()?;
        if pcm_len % frame_bytes != 0 {
            return None;
        }
        (pcm_len / frame_bytes)
            .checked_mul(1_000)?
            .checked_div(u64::from(sample_rate_hz))
    }
}

impl sealed::GeneratedAudioSink for GeneratedAudioBuffer {}

impl GeneratedAudioSink for GeneratedAudioBuffer {
    fn push_generated_audio(
        &mut self,
        pcm: &[u8],
        sample_rate_hz: u32,
        channels: u16,
        sample_format: PcmSampleFormat,
    ) -> Result<(), ProviderError> {
        let same_format = self
            .sample_rate_hz
            .is_none_or(|value| value == sample_rate_hz)
            && self.channels.is_none_or(|value| value == channels)
            && self
                .sample_format
                .is_none_or(|value| value == sample_format);
        let frame_bytes = usize::from(channels).checked_mul(match sample_format {
            PcmSampleFormat::S16Le => 2,
        });
        if sample_rate_hz == 0
            || channels == 0
            || pcm.is_empty()
            || !same_format
            || !frame_bytes.is_some_and(|size| size > 0 && pcm.len() % size == 0)
        {
            return Err(invalid_generated_output());
        }
        let frame_bytes = frame_bytes.ok_or_else(invalid_generated_output)?;
        let prospective_len = self
            .pcm
            .len()
            .checked_add(pcm.len())
            .ok_or_else(invalid_generated_output)?;
        let prospective_duration_millis = u64::try_from(prospective_len)
            .ok()
            .and_then(|bytes| bytes.checked_div(u64::try_from(frame_bytes).ok()?))
            .and_then(|frames| frames.checked_mul(1_000))
            .and_then(|millis| millis.checked_div(u64::from(sample_rate_hz)))
            .ok_or_else(invalid_generated_output)?;
        if prospective_len > MAX_GENERATED_AUDIO_BYTES
            || prospective_duration_millis > MAX_GENERATED_AUDIO_DURATION_MILLIS
        {
            return Err(invalid_generated_output());
        }
        self.sample_rate_hz = Some(sample_rate_hz);
        self.channels = Some(channels);
        self.sample_format = Some(sample_format);
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }
}

pub trait GeneratedAudioSink: sealed::GeneratedAudioSink {
    /// Returns synthesized PCM to a sealed in-memory generation buffer.
    /// External crates cannot implement this trait, so provider callbacks cannot become transport.
    ///
    /// # Errors
    /// Returns `ProviderError` for invalid or format-changing generated audio.
    fn push_generated_audio(
        &mut self,
        pcm: &[u8],
        sample_rate_hz: u32,
        channels: u16,
        sample_format: PcmSampleFormat,
    ) -> Result<(), ProviderError>;
}

pub trait TtsPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    /// Synthesizes speech while observing the runtime cancellation authority.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or policy denial.
    fn synthesize(
        &self,
        request: &TtsRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct GeneratedVideoFrame {
    pub encoded_frame: Vec<u8>,
    pub timestamp_micros: u64,
}

impl std::fmt::Debug for GeneratedVideoFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedVideoFrame")
            .field("encoded_bytes", &self.encoded_frame.len())
            .field("timestamp_micros", &self.timestamp_micros)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct GeneratedVideoBuffer {
    frames: Vec<GeneratedVideoFrame>,
    encoded_bytes: usize,
    first_timestamp_micros: Option<u64>,
}

impl std::fmt::Debug for GeneratedVideoBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedVideoBuffer")
            .field("frame_count", &self.frames.len())
            .field("encoded_bytes", &self.encoded_bytes)
            .field("first_timestamp_micros", &self.first_timestamp_micros)
            .finish()
    }
}

impl GeneratedVideoBuffer {
    #[must_use]
    pub fn frames(&self) -> &[GeneratedVideoFrame] {
        &self.frames
    }
}

impl sealed::GeneratedVideoSink for GeneratedVideoBuffer {}

impl GeneratedVideoSink for GeneratedVideoBuffer {
    fn push_generated_frame(
        &mut self,
        encoded_frame: &[u8],
        timestamp_micros: u64,
    ) -> Result<(), ProviderError> {
        if encoded_frame.is_empty()
            || self
                .frames
                .last()
                .is_some_and(|frame| timestamp_micros < frame.timestamp_micros)
        {
            return Err(invalid_generated_output());
        }
        let prospective_frames = self
            .frames
            .len()
            .checked_add(1)
            .ok_or_else(invalid_generated_output)?;
        let prospective_bytes = self
            .encoded_bytes
            .checked_add(encoded_frame.len())
            .ok_or_else(invalid_generated_output)?;
        let first_timestamp_micros = self.first_timestamp_micros.unwrap_or(timestamp_micros);
        let duration_micros = timestamp_micros
            .checked_sub(first_timestamp_micros)
            .ok_or_else(invalid_generated_output)?;
        if prospective_frames > MAX_GENERATED_VIDEO_FRAMES
            || prospective_bytes > MAX_GENERATED_VIDEO_BYTES
            || duration_micros > MAX_GENERATED_VIDEO_DURATION_MICROS
        {
            return Err(invalid_generated_output());
        }
        self.encoded_bytes = prospective_bytes;
        self.first_timestamp_micros = Some(first_timestamp_micros);
        self.frames.push(GeneratedVideoFrame {
            encoded_frame: encoded_frame.to_vec(),
            timestamp_micros,
        });
        Ok(())
    }
}

pub trait GeneratedVideoSink: sealed::GeneratedVideoSink {
    /// Returns one encoded frame to a sealed in-memory generation buffer.
    /// External crates cannot implement this trait, so provider callbacks cannot become transport.
    ///
    /// # Errors
    /// Returns `ProviderError` when generated-frame handoff fails.
    fn push_generated_frame(
        &mut self,
        encoded_frame: &[u8],
        timestamp_micros: u64,
    ) -> Result<(), ProviderError>;
}

pub trait AvatarPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    /// Renders visual output from authorized audio input.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or policy denial.
    fn render(
        &self,
        audio: &AudioInput,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedVideoSink,
    ) -> Result<UsageEvidence, ProviderError>;
}

fn invalid_generated_output() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

#[cfg(test)]
#[path = "generated_output_tests.rs"]
mod tests;
