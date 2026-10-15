use vpr_integration::{RealtimeAvatarPort, TtsPort};
use vpr_runtime::{
    ActiveTurn, OutputDeliveryHandle, RealtimeAudioStageTimings, RealtimeAvatarHandle,
    RealtimeAvatarOutputError,
};

use super::{LabClientCommand, LabError, voice::{terminalize_failed_turn, terminalize_provider_error}};

pub(super) fn deliver_phrase(
    turn: &ActiveTurn,
    provider: &dyn vpr_integration::RealtimeAvatarPort,
    tts: Option<&dyn TtsPort>,
    handle: &RealtimeAvatarHandle,
    phrase: &str,
) -> Result<
    (
        OutputDeliveryHandle,
        Option<LabClientCommand>,
        Option<RealtimeAudioStageTimings>,
    ),
    LabError,
> {
    if let Some(tts) = tts {
        let (delivery, timings) = turn
            .synthesize_and_deliver_realtime_avatar_audio_with_timings(
                tts, provider, handle, phrase,
            )
            .map_err(|error| terminalize_avatar_output_error(turn, error))?;
        return Ok((delivery, None, Some(timings)));
    }
    let client_text = handle
        .client_control()
        .is_some_and(|control| control.text_input);
    if client_text {
        let (delivery, command) = turn
            .prepare_realtime_avatar_client_text(provider, handle, phrase)
            .map_err(|error| terminalize_avatar_output_error(turn, error))?;
        Ok((delivery, Some(command.into()), None))
    } else {
        let delivery = turn
            .deliver_realtime_avatar_text(provider, handle, phrase)
            .map_err(|error| terminalize_avatar_output_error(turn, error))?;
        Ok((delivery, None, None))
    }
}

fn terminalize_avatar_output_error(
    turn: &ActiveTurn,
    error: RealtimeAvatarOutputError,
) -> LabError {
    match error {
        RealtimeAvatarOutputError::Runtime(reason) => {
            terminalize_failed_turn(turn, LabError::Runtime(reason))
        }
        RealtimeAvatarOutputError::Provider(error) => terminalize_provider_error(turn, error),
    }
}

