use crate::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind, UsageEvidence,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRequest {
    pub locale: String,
    pub context: String,
}

mod sealed {
    pub trait GeneratedTextSink {}
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

pub trait LlmTextStream {
    /// Pulls the next generated text chunk under the caller-owned cancellation authority.
    ///
    /// Returning `Ok(None)` means the provider stream completed cleanly. Provider adapters retain
    /// protocol framing internally; callers receive generated text only.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or malformed provider framing.
    fn next_chunk(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<String>, ProviderError>;

    /// Returns usage observed so far for this provider stream.
    fn usage(&self) -> UsageEvidence;
}

pub trait LlmPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    /// Opens a pull-based provider stream without transferring Persona authority or transport
    /// ownership to the provider callback. Realtime runtimes can poll this stream and decide when
    /// authorized output may leave the runtime boundary.
    ///
    /// Providers that have not implemented pull streaming may keep the default unavailable result;
    /// non-realtime callers can continue using `stream`.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or policy denial.
    fn open_stream(
        &self,
        _request: &LlmRequest,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn LlmTextStream>, ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }

    /// Streams provider output into a sealed generation sink.
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

