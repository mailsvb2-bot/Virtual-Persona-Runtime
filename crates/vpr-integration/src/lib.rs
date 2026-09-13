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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageEvidence {
    pub input_units: Option<u64>,
    pub output_units: Option<u64>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInput {
    pub pcm: Vec<u8>,
    pub sample_rate_hz: u32,
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
        input: &AudioInput,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedAudioBuffer {
    pcm: Vec<u8>,
    sample_rate_hz: Option<u32>,
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
}

impl sealed::GeneratedAudioSink for GeneratedAudioBuffer {}

impl GeneratedAudioSink for GeneratedAudioBuffer {
    fn push_generated_audio(
        &mut self,
        pcm: &[u8],
        sample_rate_hz: u32,
    ) -> Result<(), ProviderError> {
        if sample_rate_hz == 0
            || self
                .sample_rate_hz
                .is_some_and(|existing| existing != sample_rate_hz)
        {
            return Err(invalid_generated_output());
        }
        self.sample_rate_hz = Some(sample_rate_hz);
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }
}

pub trait GeneratedAudioSink: sealed::GeneratedAudioSink {
    /// Returns synthesized audio to a sealed in-memory generation buffer.
    /// External crates cannot implement this trait, so provider callbacks cannot become transport.
    ///
    /// # Errors
    /// Returns `ProviderError` for invalid generated audio.
    fn push_generated_audio(
        &mut self,
        pcm: &[u8],
        sample_rate_hz: u32,
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
        text: &str,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedVideoFrame {
    pub encoded_frame: Vec<u8>,
    pub timestamp_micros: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedVideoBuffer {
    frames: Vec<GeneratedVideoFrame>,
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
    fn generated_text_buffer_accumulates_without_transport_contract() {
        let mut buffer = GeneratedTextBuffer::default();
        buffer.push_generated_text("При").unwrap();
        buffer.push_generated_text("вет").unwrap();
        assert_eq!(buffer.as_str(), "Привет");
    }

    #[test]
    fn generated_audio_buffer_rejects_sample_rate_changes() {
        let mut buffer = GeneratedAudioBuffer::default();
        buffer.push_generated_audio(&[1, 2], 16_000).unwrap();
        let error = buffer.push_generated_audio(&[3, 4], 24_000).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
        assert_eq!(buffer.pcm(), &[1, 2]);
        assert_eq!(buffer.sample_rate_hz(), Some(16_000));
    }

    #[test]
    fn generated_video_buffer_retains_generation_evidence_only() {
        let mut buffer = GeneratedVideoBuffer::default();
        buffer.push_generated_frame(&[7, 8], 42).unwrap();
        assert_eq!(buffer.frames().len(), 1);
        assert_eq!(buffer.frames()[0].encoded_frame, vec![7, 8]);
        assert_eq!(buffer.frames()[0].timestamp_micros, 42);
    }
}
