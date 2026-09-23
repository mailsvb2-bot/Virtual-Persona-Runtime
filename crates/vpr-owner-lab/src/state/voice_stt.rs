use vpr_integration::{
    AudioInput, ProviderError, ProviderErrorKind, SttPort, SttRequest, SttStreamEvent,
    SttStreamRequest, Transcript, UsageEvidence,
};
use vpr_runtime::{ActiveTurn, ProviderExecutionError};

use super::{LabError, voice::terminalize_provider_error};

const STT_STREAM_CHUNK_BYTES: usize = 640;

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
    match turn.open_stt_stream(stt, &request) {
        Ok(mut stream) => {
            for chunk in audio.pcm.chunks(STT_STREAM_CHUNK_BYTES) {
                stream
                    .push_audio(chunk)
                    .map_err(|error| terminalize_provider_error(turn, error))?;
            }
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
        Err(ProviderExecutionError::Provider(error))
            if error.kind == ProviderErrorKind::Unavailable && !error.retryable =>
        {
            turn.execute_stt(
                stt,
                &SttRequest {
                    audio: audio.clone(),
                    locale_hint: Some("ru-RU".to_owned()),
                },
            )
            .map_err(|error| terminalize_provider_error(turn, error))
        }
        Err(error) => Err(terminalize_provider_error(turn, error)),
    }
}
