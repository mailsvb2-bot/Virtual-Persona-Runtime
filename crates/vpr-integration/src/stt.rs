use crate::{
    AudioInput, CancellationProbe, PcmSampleFormat, ProviderDescriptor, ProviderError,
    ProviderErrorKind, UsageEvidence,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttStreamRequest {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: PcmSampleFormat,
    pub locale_hint: Option<String>,
}

impl SttStreamRequest {
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.sample_rate_hz > 0
            && self.channels > 0
            && usize::from(self.channels)
                .checked_mul(self.sample_format.bytes_per_sample())
                .is_some_and(|frame_bytes| frame_bytes > 0)
    }

    #[must_use]
    pub fn is_well_formed_chunk(&self, pcm: &[u8]) -> bool {
        if !self.is_well_formed() || pcm.is_empty() {
            return false;
        }
        usize::from(self.channels)
            .checked_mul(self.sample_format.bytes_per_sample())
            .is_some_and(|frame_bytes| pcm.len() % frame_bytes == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SttStreamEvent {
    Interim(Transcript),
    Final(Transcript),
}

impl SttStreamEvent {
    #[must_use]
    pub const fn transcript(&self) -> &Transcript {
        match self {
            Self::Interim(transcript) | Self::Final(transcript) => transcript,
        }
    }

    #[must_use]
    pub const fn is_final(&self) -> bool {
        matches!(self, Self::Final(_))
    }
}

pub trait SttAudioStream: Send {
    /// Sends one provider-neutral PCM chunk to the active recognition stream.
    ///
    /// # Errors
    /// Returns a typed provider failure for malformed audio, cancellation, or transport failure.
    fn push_audio(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Signals that no more input audio will be sent.
    ///
    /// # Errors
    /// Returns a typed provider failure when the stream cannot finalize input.
    fn finish_input(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError>;

    /// Pulls the next normalized recognition event.
    ///
    /// Returning Ok(None) means the provider stream completed cleanly. Provider protocol framing
    /// and credentials remain inside the adapter.
    ///
    /// # Errors
    /// Returns a typed provider failure for cancellation, malformed framing, or transport failure.
    fn next_event(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<SttStreamEvent>, ProviderError>;

    #[must_use]
    fn usage(&self) -> UsageEvidence;
}

pub trait SttPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    /// Opens one full-duplex recognition stream while keeping provider transport details behind
    /// the integration boundary. Batch-only adapters may keep the default unavailable result.
    ///
    /// # Errors
    /// Returns a typed provider failure, including cancellation or policy denial.
    fn open_stream(
        &self,
        _request: &SttStreamRequest,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn SttAudioStream>, ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }

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
