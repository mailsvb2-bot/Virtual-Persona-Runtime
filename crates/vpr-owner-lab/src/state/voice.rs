use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use vpr_domain::{Rt0ReasonCode, TurnState};
pub use vpr_evaluation::SessionUsageEvidence as LabProviderUsage;
pub type LabVoiceUsage = LabProviderUsage;
use vpr_integration::{
    AudioInput, LlmPort, LlmRequest, PcmSampleFormat, SttPort, TimedGeneratedTextBuffer,
    UsageEvidence,
};
use vpr_runtime::{
    ActiveTurn, OutputDeliveryHandle, ProviderExecutionError, RealtimeAvatarHandle,
    RealtimeAvatarOutputError, TurnInterruptHandle,
};

use super::{
    LabClientCommand, LabError, OwnerLabEngine, map_provider_execution,
    voice_input::LabVoiceInput, voice_phrase::RealtimePhraseBuffer,
    voice_stt::transcribe_voice_audio,
};

const VOICE_SAMPLE_RATE_HZ: u32 = 16_000;
const VOICE_CHANNELS: u16 = 1;
const MAX_VOICE_MILLIS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabVoiceSegment {
    pub evidence_turn_sequence: u64,
    pub evidence_output_sequence: u64,
    pub client_command: Option<LabClientCommand>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabVoiceResult {
    pub transcript: String,
    pub reply: String,
    pub locale: String,
    pub evidence_turn_sequence: u64,
    pub evidence_output_sequence: u64,
    pub stt_millis: u64,
    pub llm_millis: u64,
    pub llm_first_meaningful_millis: u64,
    pub avatar_millis: u64,
    pub total_millis: u64,
    pub stt_usage: LabVoiceUsage,
    pub llm_usage: LabVoiceUsage,
    pub client_command: Option<LabClientCommand>,
}

enum VoiceOutputMode<'a> {
    Buffered,
    Streaming(&'a mut dyn FnMut(LabVoiceSegment) -> Result<(), LabError>),
}

struct VoiceGeneration {
    reply: String,
    usage: UsageEvidence,
    llm_millis: u64,
    first_meaningful_millis: u64,
    avatar_millis: u64,
    output_sequences: Vec<u64>,
    client_command: Option<LabClientCommand>,
}

impl OwnerLabEngine {
    #[must_use]
    pub fn with_stt(mut self, stt: Box<dyn SttPort>) -> Self {
        self.stt = Some(stt);
        self
    }

    #[must_use]
    pub fn with_llm(mut self, llm: Box<dyn LlmPort>) -> Self {
        self.llm = Some(llm);
        self
    }

    #[must_use]
    pub fn with_voice(self, stt: Box<dyn SttPort>, llm: Box<dyn LlmPort>) -> Self {
        self.with_stt(stt).with_llm(llm)
    }

    /// Runs one microphone utterance through STT -> LLM -> realtime avatar on one canonical turn.
    ///
    /// This compatibility path keeps a single buffered avatar delivery. Interactive Owner Lab
    /// should use `voice_turn_streaming` so first useful output does not wait for full generation.
    ///
    /// # Errors
    /// Fails closed when voice providers are not configured, audio is malformed, session state is
    /// not active, authority changes, cancellation occurs, or any provider fails.
    pub fn voice_turn(
        &mut self,
        pcm_s16le_mono_16khz: Vec<u8>,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
    ) -> Result<LabVoiceResult, LabError> {
        self.run_voice_turn(
            pcm_s16le_mono_16khz,
            register_interrupt,
            VoiceOutputMode::Buffered,
        )
    }

    /// Runs one realtime microphone turn using the canonical pull-based LLM stream.
    ///
    /// Generated text is converted into bounded natural phrase segments while the provider is
    /// still generating. Each phrase receives its own runtime-issued output-delivery handle before
    /// the opaque browser/provider command is emitted. The same turn cancellation authority covers
    /// STT, the open LLM stream, every avatar segment and the remaining generation tail.
    ///
    /// # Errors
    /// Fails closed for malformed audio, unavailable pull streaming, authority/cancellation
    /// changes, provider failures, or a failed segment handoff.
    pub fn voice_turn_streaming(
        &mut self,
        pcm_s16le_mono_16khz: Vec<u8>,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
        mut emit_segment: impl FnMut(LabVoiceSegment) -> Result<(), LabError>,
    ) -> Result<LabVoiceResult, LabError> {
        self.run_voice_turn(
            pcm_s16le_mono_16khz,
            register_interrupt,
            VoiceOutputMode::Streaming(&mut emit_segment),
        )
    }

    /// Finishes an incrementally uploaded microphone turn after browser input closes.
    ///
    /// STT remains bound to the same canonical turn that accepted live PCM; only the finalization
    /// and downstream LLM/avatar phases begin here. This keeps interruption, revocation and egress
    /// authority continuous across microphone -> STT -> LLM -> avatar.
    ///
    /// # Errors
    /// Fails closed for empty input, provider/runtime cancellation, invalid session state, or a
    /// failed output segment handoff.
    pub fn finish_voice_input_streaming(
        &mut self,
        input: LabVoiceInput,
        mut emit_segment: impl FnMut(LabVoiceSegment) -> Result<(), LabError>,
    ) -> Result<LabVoiceResult, LabError> {
        if input.received_bytes() == 0 {
            return Err(input.abort(LabError::InvalidInput));
        }
        let (turn, evidence_turn_sequence, stt_input, _) = input.into_parts();
        let total_started = Instant::now();
        let stt_started = Instant::now();
        let stt = self.stt.as_ref().ok_or(LabError::InvalidState)?;
        let (transcript, stt_usage) = stt_input.finish(&turn, stt.as_ref())?;
        let stt_millis = elapsed_millis(stt_started);
        let llm_context = self.conversation_context(&transcript.text)?;
        let request = LlmRequest {
            locale: transcript.locale.clone(),
            context: llm_context,
        };
        let llm = self.llm.as_ref().ok_or(LabError::InvalidState)?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        let generation = self.streaming_generation(
            &turn,
            llm.as_ref(),
            handle,
            &request,
            evidence_turn_sequence,
            &mut emit_segment,
        )?;
        let evidence_output_sequence = generation
            .output_sequences
            .first()
            .copied()
            .ok_or_else(|| terminalize_failed_turn(&turn, LabError::InvalidInput))?;
        turn.complete().map_err(LabError::Runtime)?;

        Ok(LabVoiceResult {
            transcript: transcript.text,
            reply: generation.reply,
            locale: transcript.locale,
            evidence_turn_sequence,
            evidence_output_sequence,
            stt_millis,
            llm_millis: generation.llm_millis,
            llm_first_meaningful_millis: generation.first_meaningful_millis,
            avatar_millis: generation.avatar_millis,
            total_millis: elapsed_millis(total_started),
            stt_usage: map_usage(&stt_usage),
            llm_usage: map_usage(&generation.usage),
            client_command: generation.client_command,
        })
    }

    fn run_voice_turn(
        &mut self,
        pcm_s16le_mono_16khz: Vec<u8>,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
        mut output_mode: VoiceOutputMode<'_>,
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

        let turn = Arc::new(self.new_turn()?);
        let evidence_turn_sequence = self.turn_counter;
        register_interrupt(turn.interrupt_handle());
        let stt = self.stt.as_ref().ok_or(LabError::InvalidState)?;
        let llm = self.llm.as_ref().ok_or(LabError::InvalidState)?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;

        let total_started = Instant::now();
        let stt_started = Instant::now();
        let (transcript, stt_usage) = transcribe_voice_audio(&turn, stt.as_ref(), &audio)?;
        let stt_millis = elapsed_millis(stt_started);
        let llm_context = self.conversation_context(&transcript.text)?;
        let request = LlmRequest {
            locale: transcript.locale.clone(),
            context: llm_context,
        };

        let generation = match &mut output_mode {
            VoiceOutputMode::Buffered => self.buffered_generation(
                &turn,
                llm.as_ref(),
                handle,
                &request,
                evidence_turn_sequence,
            )?,
            VoiceOutputMode::Streaming(emit_segment) => self.streaming_generation(
                &turn,
                llm.as_ref(),
                handle,
                &request,
                evidence_turn_sequence,
                emit_segment,
            )?,
        };

        let evidence_output_sequence = generation
            .output_sequences
            .first()
            .copied()
            .ok_or_else(|| terminalize_failed_turn(&turn, LabError::InvalidInput))?;
        turn.complete().map_err(LabError::Runtime)?;

        Ok(LabVoiceResult {
            transcript: transcript.text,
            reply: generation.reply,
            locale: transcript.locale,
            evidence_turn_sequence,
            evidence_output_sequence,
            stt_millis,
            llm_millis: generation.llm_millis,
            llm_first_meaningful_millis: generation.first_meaningful_millis,
            avatar_millis: generation.avatar_millis,
            total_millis: elapsed_millis(total_started),
            stt_usage: map_usage(&stt_usage),
            llm_usage: map_usage(&generation.usage),
            client_command: generation.client_command,
        })
    }

    fn buffered_generation(
        &self,
        turn: &Arc<ActiveTurn>,
        llm: &dyn LlmPort,
        handle: &RealtimeAvatarHandle,
        request: &LlmRequest,
        evidence_turn_sequence: u64,
    ) -> Result<VoiceGeneration, LabError> {
        let llm_started = Instant::now();
        let mut generated = TimedGeneratedTextBuffer::start();
        let llm_usage = turn
            .execute_llm(llm, request, &mut generated)
            .map_err(|error| terminalize_provider_error(turn, error))?;
        let llm_millis = elapsed_millis(llm_started);
        let (reply, first_meaningful) = generated.into_parts();
        let first_meaningful = first_meaningful
            .ok_or_else(|| terminalize_failed_turn(turn, LabError::InvalidInput))?;
        if reply.trim().is_empty() {
            return Err(terminalize_failed_turn(turn, LabError::InvalidInput));
        }

        turn.begin_output().map_err(LabError::Runtime)?;
        let avatar_started = Instant::now();
        let (delivery, client_command) =
            deliver_phrase(turn, self.provider.as_ref(), handle, &reply)?;
        let output_sequence =
            self.voice_playback
                .register_delivery(evidence_turn_sequence, turn, delivery)?;
        let avatar_millis = elapsed_millis(avatar_started);
        Ok(VoiceGeneration {
            reply,
            usage: llm_usage,
            llm_millis,
            first_meaningful_millis: first_meaningful,
            avatar_millis,
            output_sequences: vec![output_sequence],
            client_command,
        })
    }

    fn streaming_generation(
        &self,
        turn: &Arc<ActiveTurn>,
        llm: &dyn LlmPort,
        handle: &RealtimeAvatarHandle,
        request: &LlmRequest,
        evidence_turn_sequence: u64,
        emit_segment: &mut dyn FnMut(LabVoiceSegment) -> Result<(), LabError>,
    ) -> Result<VoiceGeneration, LabError> {
        let llm_started = Instant::now();
        let mut stream = turn
            .open_llm_stream(llm, request)
            .map_err(|error| terminalize_provider_error(turn, error))?;
        let mut reply = String::new();
        let mut first_meaningful = None;
        let mut phrases = RealtimePhraseBuffer::default();
        let mut output_sequences = Vec::new();
        let mut avatar_millis = 0_u64;
        let mut output_started = false;

        while let Some(chunk) = stream
            .next_chunk()
            .map_err(|error| terminalize_provider_error(turn, error))?
        {
            if first_meaningful.is_none() && !chunk.trim().is_empty() {
                first_meaningful = Some(elapsed_millis(llm_started));
            }
            reply.push_str(&chunk);
            for phrase in phrases.push(&chunk) {
                if !output_started {
                    turn.begin_output().map_err(LabError::Runtime)?;
                    output_started = true;
                }
                let avatar_started = Instant::now();
                let (delivery, client_command) =
                    deliver_phrase(turn, self.provider.as_ref(), handle, &phrase)?;
                avatar_millis = avatar_millis.saturating_add(elapsed_millis(avatar_started));
                let output_sequence = self.voice_playback.register_delivery(
                    evidence_turn_sequence,
                    turn,
                    delivery,
                )?;
                let segment = LabVoiceSegment {
                    evidence_turn_sequence,
                    evidence_output_sequence: output_sequence,
                    client_command,
                };
                emit_segment(segment).map_err(|error| terminalize_failed_turn(turn, error))?;
                output_sequences.push(output_sequence);
            }
        }

        if let Some(phrase) = phrases.finish() {
            if !output_started {
                turn.begin_output().map_err(LabError::Runtime)?;
            }
            let avatar_started = Instant::now();
            let (delivery, client_command) =
                deliver_phrase(turn, self.provider.as_ref(), handle, &phrase)?;
            avatar_millis = avatar_millis.saturating_add(elapsed_millis(avatar_started));
            let output_sequence =
                self.voice_playback
                    .register_delivery(evidence_turn_sequence, turn, delivery)?;
            let segment = LabVoiceSegment {
                evidence_turn_sequence,
                evidence_output_sequence: output_sequence,
                client_command,
            };
            emit_segment(segment).map_err(|error| terminalize_failed_turn(turn, error))?;
            output_sequences.push(output_sequence);
        }

        let llm_millis = elapsed_millis(llm_started);
        let first_meaningful = first_meaningful
            .ok_or_else(|| terminalize_failed_turn(turn, LabError::InvalidInput))?;
        if reply.trim().is_empty() || output_sequences.is_empty() {
            return Err(terminalize_failed_turn(turn, LabError::InvalidInput));
        }
        Ok(VoiceGeneration {
            reply,
            usage: stream.usage(),
            llm_millis,
            first_meaningful_millis: first_meaningful,
            avatar_millis,
            output_sequences,
            client_command: None,
        })
    }
}

impl OwnerLabEngine {
    /// Reconciles a browser-confirmed client-transport send with one exact canonical segment.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for stale or mismatched evidence identifiers.
    pub fn acknowledge_voice_delivery_sent(
        &mut self,
        evidence_turn_sequence: u64,
        evidence_output_sequence: u64,
    ) -> Result<(), LabError> {
        self.voice_playback
            .acknowledge_voice_delivery_sent(evidence_turn_sequence, evidence_output_sequence)
    }

    /// Reconciles browser-observed remote audio with one exact canonical segment.
    ///
    /// Duplicate acknowledgements are idempotent; stale or unknown turn/segment sequences fail
    /// closed.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when the segment does not belong to the current session.
    pub fn acknowledge_voice_playback(
        &mut self,
        evidence_turn_sequence: u64,
        evidence_output_sequence: u64,
    ) -> Result<(), LabError> {
        if self.session_audience.is_none()
            || !self
                .session
                .as_ref()
                .is_some_and(|session| session.state() == vpr_domain::RealtimeSessionState::Active)
        {
            return Err(LabError::InvalidState);
        }
        self.voice_playback
            .acknowledge_voice_playback(evidence_turn_sequence, evidence_output_sequence)
    }
}

fn deliver_phrase(
    turn: &ActiveTurn,
    provider: &dyn vpr_integration::RealtimeAvatarPort,
    handle: &RealtimeAvatarHandle,
    phrase: &str,
) -> Result<(OutputDeliveryHandle, Option<LabClientCommand>), LabError> {
    let client_text = handle
        .client_control()
        .is_some_and(|control| control.text_input);
    if client_text {
        let (delivery, command) = turn
            .prepare_realtime_avatar_client_text(provider, handle, phrase)
            .map_err(|error| terminalize_avatar_output_error(turn, error))?;
        Ok((delivery, Some(command.into())))
    } else {
        let delivery = turn
            .deliver_realtime_avatar_text(provider, handle, phrase)
            .map_err(|error| terminalize_avatar_output_error(turn, error))?;
        Ok((delivery, None))
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

pub(super) fn terminalize_provider_error(
    turn: &ActiveTurn,
    error: ProviderExecutionError,
) -> LabError {
    let mapped = map_provider_execution(error);
    terminalize_failed_turn(turn, mapped)
}

pub(super) fn terminalize_failed_turn(turn: &ActiveTurn, error: LabError) -> LabError {
    match turn.state() {
        TurnState::Processing | TurnState::Outputting => match turn.fail() {
            Ok(()) => error,
            Err(_) if turn.state() == TurnState::Cancelled => {
                LabError::Runtime(Rt0ReasonCode::TurnCancelled)
            }
            Err(reason) => LabError::Runtime(reason),
        },
        TurnState::Cancelled => LabError::Runtime(Rt0ReasonCode::TurnCancelled),
        _ => error,
    }
}

pub(super) fn map_usage(usage: &UsageEvidence) -> LabVoiceUsage {
    LabVoiceUsage {
        input_units: usage.input_units,
        output_units: usage.output_units,
        estimated_cost_microunits: usage.estimated_cost_microunits,
        provider_charge_microunits: usage.provider_charge_microunits,
    }
}

pub(super) fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
