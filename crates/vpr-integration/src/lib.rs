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

pub trait TextSink {
    /// Delivers a generated text chunk to the downstream transport.
    ///
    /// # Errors
    /// Returns `ProviderError` when downstream delivery fails or is cancelled.
    fn push_text(&mut self, chunk: &str) -> Result<(), ProviderError>;
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
        sink: &mut dyn TextSink,
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

pub trait AudioSink {
    /// Delivers synthesized audio to the downstream media path.
    ///
    /// # Errors
    /// Returns `ProviderError` when delivery fails or is cancelled.
    fn push_audio(&mut self, pcm: &[u8], sample_rate_hz: u32) -> Result<(), ProviderError>;
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
        sink: &mut dyn AudioSink,
    ) -> Result<UsageEvidence, ProviderError>;
}

pub trait VideoSink {
    /// Delivers one encoded frame on the canonical session timeline.
    ///
    /// # Errors
    /// Returns `ProviderError` when frame delivery fails or is cancelled.
    fn push_frame(
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
        sink: &mut dyn VideoSink,
    ) -> Result<UsageEvidence, ProviderError>;
}
