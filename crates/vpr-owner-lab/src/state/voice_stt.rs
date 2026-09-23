use vpr_integration::{
    AudioInput, ProviderError, ProviderErrorKind, SttPort, SttRequest, SttStreamEvent,
    SttStreamRequest, Transcript, UsageEvidence,
};
use vpr_runtime::{ActiveTurn, AuthorizedSttStream, ProviderExecutionError};

use super::{LabError, voice::terminalize_provider_error};

const STT_STREAM_CHUNK_BYTES: usize = 640;

enum VoiceSttMode {
    Streaming(AuthorizedSttStream),
    Buffered(Vec<u8>),
}

pub(super) struct VoiceSttInput {
    request: SttStreamRequest,
    mode: VoiceSttMode,
}

impl VoiceSttInput {
    pub(super) fn open(
        turn: &ActiveTurn,
        stt: &dyn SttPort,
        request: SttStreamRequest,
    ) -> Result<Self, LabError> {
        match turn.open_stt_stream(stt, &request) {
            Ok(stream) => Ok(Self {
                request,
                mode: VoiceSttMode::Streaming(stream),
            }),
            Err(ProviderExecutionError::Provider(error))
                if error.kind == ProviderErrorKind::Unavailable && !error.retryable =>
            {
                Ok(Self {
                    request,
                    mode: VoiceSttMode::Buffered(Vec::new()),
                })
            }
            Err(error) => Err(terminalize_provider_error(turn, error)),
        }
    }

    pub(super) fn push_audio(
        &mut self,
        turn: &ActiveTurn,
        pcm: &[u8],
    ) -> Result<(), LabError> {
        if !self.request.is_well_formed_chunk(pcm) {
            return Err(super::voice::terminalize_failed_turn(
                turn,
                LabError::InvalidInput,
            ));
        }
        match &mut self.mode {
            VoiceSttMode::Streaming(stream) => {
                for chunk in pcm.chunks(STT_STREAM_CHUNK_BYTES) {
                    stream
                        .push_audio(chunk)
                        .map_err(|error| terminalize_provider_error(turn, error))?;
                }
            }
            VoiceSttMode::Buffered(buffer) => buffer.extend_from_slice(pcm),
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        turn: &ActiveTurn,
        stt: &dyn SttPort,
    ) -> Result<(Transcript, UsageEvidence), LabError> {
        match self.mode {
            VoiceSttMode::Streaming(mut stream) => {
                stream
                    .finish_input()
                    .map_err(|error| terminalize_provider_error(turn, error))?;
                let mut final_transcript = None;
                while let Some(event) = stream
                    .next_event()
                    .map_err(|error| terminalize_provider_error(turn, error))?
                {
                    if let SttStreamEvent::Final(transcript) = event
                        && !transcript.text.trim().is_empty()
                    {
                        final_transcript = Some(transcript);
                    }
                }
                let usage = stream.usage();
                let transcript = final_transcript.ok_or_else(|| {
                    terminalize_provider_error(
                        turn,
                        ProviderExecutionError::Provider(ProviderError {
                            kind: ProviderErrorKind::InvalidResponse,
                            retryable: false,
                        }),
                    )
                })?;
                Ok((transcript, usage))
            }
            VoiceSttMode::Buffered(pcm) => {
                let audio = AudioInput {
                    pcm,
                    sample_rate_hz: self.request.sample_rate_hz,
                    channels: self.request.channels,
                    sample_format: self.request.sample_format,
                };
                turn.execute_stt(
                    stt,
                    &SttRequest {
                        audio,
                        locale_hint: self.request.locale_hint,
                    },
                )
                .map_err(|error| terminalize_provider_error(turn, error))
            }
        }
    }
}

pub(super) fn transcribe_voice_audio(
    turn: &ActiveTurn,
    stt: &dyn SttPort,
    audio: &AudioInput,
) -> Result<(Transcript, UsageEvidence), LabError> {
    let request = SttStreamRequest {
        sample_rate_hz: audio.sample_rate_hz,
        channels: audio.channels,
        sample_format: audio.sample_format,
        locale_hint: Some("ru-RU".to_owned()),
    };
    let mut input = VoiceSttInput::open(turn, stt, request)?;
    input.push_audio(turn, &audio.pcm)?;
    input.finish(turn, stt)
}
