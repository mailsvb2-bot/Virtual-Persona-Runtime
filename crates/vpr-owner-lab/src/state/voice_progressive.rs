use std::time::Instant;

use serde::Serialize;
use vpr_integration::{AudioInput, LlmRequest, PcmSampleFormat, SttRequest};
use vpr_runtime::{OutputDeliveryHandle, TurnInterruptHandle};

use super::voice::{
    LabVoiceResult, MAX_VOICE_MILLIS, PendingVoicePlayback, VOICE_CHANNELS, VOICE_SAMPLE_RATE_HZ,
    elapsed_millis, map_usage, terminalize_avatar_output_error, terminalize_failed_turn,
    terminalize_provider_error,
};
use super::{LabClientCommand, LabError, OwnerLabEngine};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabVoicePhrase {
    pub evidence_turn_sequence: u64,
    pub evidence_output_sequence: u64,
    pub client_command: Option<LabClientCommand>,
}

impl OwnerLabEngine {
    /// Runs one microphone utterance through STT -> incremental LLM phrases -> realtime avatar.
    ///
    /// The phrase callback fires as soon as each runtime-authorized spoken phrase has a canonical
    /// output segment. Browser-owned transports can forward the command immediately instead of
    /// waiting for the full LLM response.
    ///
    /// # Errors
    /// Fails closed on malformed audio, missing providers, authority/cancellation changes,
    /// provider failure, empty generation, avatar delivery failure, or phrase handoff failure.
    pub fn voice_turn_progressive(
        &mut self,
        pcm_s16le_mono_16khz: Vec<u8>,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
        mut on_phrase: impl FnMut(&LabVoicePhrase) -> Result<(), LabError>,
    ) -> Result<LabVoiceResult, LabError> {
        let audio = AudioInput {
            pcm: pcm_s16le_mono_16khz,
            sample_rate_hz: VOICE_SAMPLE_RATE_HZ,
            channels: VOICE_CHANNELS,
            sample_format: PcmSampleFormat::S16Le,
        };
        let duration = audio.duration_millis().ok_or(LabError::InvalidInput)?;
        if duration == 0 || duration > MAX_VOICE_MILLIS {
            return Err(LabError::InvalidInput);
        }
        if self.stt.is_none() || self.llm.is_none() || self.avatar.is_none() {
            return Err(LabError::InvalidState);
        }

        let turn = self.new_turn()?;
        let evidence_turn_sequence = self.turn_counter;
        register_interrupt(turn.interrupt_handle());
        let stt = self.stt.as_ref().ok_or(LabError::InvalidState)?;
        let llm = self.llm.as_ref().ok_or(LabError::InvalidState)?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;

        let total_started = Instant::now();
        let stt_started = Instant::now();
        let (transcript, stt_usage) = turn
            .execute_stt(
                stt.as_ref(),
                &SttRequest {
                    audio,
                    locale_hint: Some("ru-RU".to_owned()),
                },
            )
            .map_err(|error| terminalize_provider_error(&turn, error))?;
        let stt_millis = elapsed_millis(stt_started);

        let llm_context = self.conversation_context(&transcript.text)?;
        let llm_started = Instant::now();
        let mut spoken = turn
            .open_spoken_llm_stream(
                llm.as_ref(),
                &LlmRequest {
                    locale: transcript.locale.clone(),
                    context: llm_context,
                },
            )
            .map_err(|error| terminalize_provider_error(&turn, error))?;
        turn.begin_output().map_err(LabError::Runtime)?;

        let client_text = handle
            .client_control()
            .is_some_and(|control| control.text_input);
        let mut reply = String::new();
        let mut deliveries: Vec<OutputDeliveryHandle> = Vec::new();
        let mut first_output_sequence = None;
        let mut avatar_millis = 0_u64;

        while let Some(phrase) = spoken
            .next_phrase()
            .map_err(|error| terminalize_provider_error(&turn, error))?
        {
            if !reply.is_empty() {
                reply.push(' ');
            }
            reply.push_str(&phrase);

            let avatar_started = Instant::now();
            let (delivery, client_command) = if client_text {
                let (delivery, command) = turn
                    .prepare_realtime_avatar_client_text(
                        self.provider.as_ref(),
                        handle,
                        &phrase,
                    )
                    .map_err(|error| terminalize_avatar_output_error(&turn, error))?;
                (delivery, Some(command.into()))
            } else {
                let delivery = turn
                    .deliver_realtime_avatar_text(self.provider.as_ref(), handle, &phrase)
                    .map_err(|error| terminalize_avatar_output_error(&turn, error))?;
                (delivery, None)
            };
            avatar_millis = avatar_millis.saturating_add(elapsed_millis(avatar_started));
            let evidence_output_sequence = delivery.sequence();
            first_output_sequence.get_or_insert(evidence_output_sequence);
            deliveries.push(delivery);

            on_phrase(&LabVoicePhrase {
                evidence_turn_sequence,
                evidence_output_sequence,
                client_command,
            })
            .map_err(|error| terminalize_failed_turn(&turn, error))?;
        }

        let llm_millis = elapsed_millis(llm_started);
        let llm_first_meaningful_millis = spoken
            .first_meaningful_elapsed_millis()
            .ok_or_else(|| terminalize_failed_turn(&turn, LabError::InvalidInput))?;
        let llm_usage = spoken.usage();
        let evidence_output_sequence = first_output_sequence
            .ok_or_else(|| terminalize_failed_turn(&turn, LabError::InvalidInput))?;
        if reply.trim().is_empty() {
            return Err(terminalize_failed_turn(&turn, LabError::InvalidInput));
        }

        turn.complete().map_err(LabError::Runtime)?;
        self.pending_voice_playback.insert(
            evidence_turn_sequence,
            PendingVoicePlayback { turn, deliveries },
        );

        Ok(LabVoiceResult {
            transcript: transcript.text,
            reply,
            locale: transcript.locale,
            evidence_turn_sequence,
            evidence_output_sequence,
            stt_millis,
            llm_millis,
            llm_first_meaningful_millis,
            avatar_millis,
            total_millis: elapsed_millis(total_started),
            stt_usage: map_usage(&stt_usage),
            llm_usage: map_usage(&llm_usage),
            client_command: None,
        })
    }
}
