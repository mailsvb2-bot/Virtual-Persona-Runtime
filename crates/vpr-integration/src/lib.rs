mod avatar;
mod transport;

pub use avatar::{
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, WebRtcIceCandidate, WebRtcIceServer, WebRtcSessionDescription,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRequest {
    pub locale: String,
    pub context: String,
}

mod sealed {
    pub trait GeneratedTextSink {}
    pub trait GeneratedAudioSink {}
    pub trait GeneratedVideoSink {}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedTextBuffer(String);

#[derive(Debug)]
pub struct TimedGeneratedTextBuffer {
    text: String,
    started: std::time::Instant,
    first_meaningful_elapsed_millis: Option<u64>,
}

impl TimedGeneratedTextBuffer {
    #[must_use]
    pub fn start() -> Self {
        Self {
            text: String::new(),
            started: std::time::Instant::now(),
            first_meaningful_elapsed_millis: None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn first_meaningful_elapsed_millis(&self) -> Option<u64> {
        self.first_meaningful_elapsed_millis
    }

    #[must_use]
    pub fn into_parts(self) -> (String, Option<u64>) {
        (self.text, self.first_meaningful_elapsed_millis)
    }
}

impl sealed::GeneratedTextSink for TimedGeneratedTextBuffer {}

impl GeneratedTextSink for TimedGeneratedTextBuffer {
    fn push_generated_text(&mut self, chunk: &str) -> Result<(), ProviderError> {
        self.text.push_str(chunk);
        if self.first_meaningful_elapsed_millis.is_none() && !self.text.trim().is_empty() {
            self.first_meaningful_elapsed_millis =
                u64::try_from(self.started.elapsed().as_millis()).ok();
        }
        Ok(())
    }
}

impl GeneratedTextBuffer {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl sealed::GeneratedTextSink for GeneratedTextBuffer {}

impl GeneratedTextSink for GeneratedTextBuffer {
    fn push_generated_text(&mut self, chunk: &str) -> Result<(), ProviderError> {
        self.0.push_str(chunk);
        Ok(())
    }
}

pub trait GeneratedTextSink: sealed::GeneratedTextSink {
    /// Returns a generated text chunk to a sealed in-memory generation buffer.
    /// External crates cannot implement this trait, so provider callbacks cannot become transport.
    ///
    /// # Errors
    /// Returns `ProviderError` when generated-text handoff fails or is cancelled.
    fn push_generated_text(&mut self, chunk: &str) -> Result<(), ProviderError>;
}

pub trait LlmPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    /// Streams provider output without transferring canonical Persona authority.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or policy denial.
    fn stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmSampleFormat {
    S16Le,
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
        match self.sample_format {
            PcmSampleFormat::S16Le => 2,
        }
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
pub struct SttRequest {
    pub audio: AudioInput,
    pub locale_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub locale: String,
}

pub trait SttPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    /// Transcribes audio using the selected provider representation.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or invalid output.
    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError>;
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
}

impl std::fmt::Debug for GeneratedVideoBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedVideoBuffer")
            .field("frame_count", &self.frames.len())
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
mod tests {
    use super::*;

    #[test]
    fn pcm_audio_reports_provider_neutral_duration() {
        let audio = AudioInput {
            pcm: vec![0; 640],
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
        };
        assert!(audio.is_well_formed());
        assert_eq!(audio.duration_millis(), Some(20));
    }

    #[test]
    fn pcm_audio_rejects_partial_frames() {
        let audio = AudioInput {
            pcm: vec![0; 3],
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
        };
        assert!(!audio.is_well_formed());
        assert_eq!(audio.duration_millis(), None);
    }

    #[test]
    fn generated_text_buffer_accumulates_without_transport_contract() {
        let mut buffer = GeneratedTextBuffer::default();
        buffer.push_generated_text("При").unwrap();
        buffer.push_generated_text("вет").unwrap();
        assert_eq!(buffer.as_str(), "Привет");
    }

    #[test]
    fn timed_generated_text_marks_only_first_meaningful_chunk() {
        let mut buffer = TimedGeneratedTextBuffer::start();
        buffer.push_generated_text("   ").unwrap();
        assert_eq!(buffer.first_meaningful_elapsed_millis(), None);
        buffer.push_generated_text("Привет").unwrap();
        let first = buffer.first_meaningful_elapsed_millis().unwrap();
        buffer.push_generated_text("!").unwrap();
        assert_eq!(buffer.as_str(), "   Привет!");
        assert_eq!(buffer.first_meaningful_elapsed_millis(), Some(first));
    }

    #[test]
    fn generated_audio_buffer_rejects_sample_rate_changes() {
        let mut buffer = GeneratedAudioBuffer::default();
        buffer
            .push_generated_audio(&[1, 2], 16_000, 1, PcmSampleFormat::S16Le)
            .unwrap();
        let error = buffer
            .push_generated_audio(&[3, 4], 24_000, 1, PcmSampleFormat::S16Le)
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
        assert_eq!(buffer.pcm(), &[1, 2]);
        assert_eq!(buffer.sample_rate_hz(), Some(16_000));
        assert_eq!(buffer.channels(), Some(1));
        assert_eq!(buffer.sample_format(), Some(PcmSampleFormat::S16Le));
    }

    #[test]
    fn generated_video_buffer_retains_generation_evidence_only() {
        let mut buffer = GeneratedVideoBuffer::default();
        buffer.push_generated_frame(&[7, 8], 42).unwrap();
        assert_eq!(buffer.frames().len(), 1);
        assert_eq!(buffer.frames()[0].encoded_frame, vec![7, 8]);
        assert_eq!(buffer.frames()[0].timestamp_micros, 42);
    }

    #[test]
    fn media_debug_output_redacts_raw_payload_bytes() {
        let audio = AudioInput {
            pcm: vec![222, 173, 190, 239],
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
        };
        let mut generated_audio = GeneratedAudioBuffer::default();
        generated_audio
            .push_generated_audio(&[222, 173, 190, 239], 16_000, 1, PcmSampleFormat::S16Le)
            .unwrap();
        let mut generated_video = GeneratedVideoBuffer::default();
        generated_video
            .push_generated_frame(&[222, 173, 190, 239], 42)
            .unwrap();

        for debug in [
            format!("{audio:?}"),
            format!("{generated_audio:?}"),
            format!("{:?}", generated_video.frames()[0]),
            format!("{generated_video:?}"),
        ] {
            assert!(!debug.contains("222"));
            assert!(!debug.contains("173"));
            assert!(!debug.contains("190"));
            assert!(!debug.contains("239"));
        }
    }

    #[test]
    fn generated_video_buffer_rejects_empty_or_decreasing_frames() {
        let mut buffer = GeneratedVideoBuffer::default();
        assert_eq!(
            buffer.push_generated_frame(&[], 1).unwrap_err().kind,
            ProviderErrorKind::InvalidResponse
        );
        buffer.push_generated_frame(&[1], 20).unwrap();
        assert_eq!(
            buffer.push_generated_frame(&[2], 19).unwrap_err().kind,
            ProviderErrorKind::InvalidResponse
        );
        assert_eq!(buffer.frames().len(), 1);
    }
}
