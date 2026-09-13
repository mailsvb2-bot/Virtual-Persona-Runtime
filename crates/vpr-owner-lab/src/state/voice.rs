use std::time::Instant;

use serde::Serialize;
use vpr_domain::{Rt0ReasonCode, TurnState};
use vpr_integration::{
    AudioInput, GeneratedTextBuffer, LlmPort, LlmRequest, PcmSampleFormat, SttPort, SttRequest,
    UsageEvidence,
};
use vpr_runtime::{ActiveTurn, ProviderExecutionError, TurnInterruptHandle};

use super::{LabError, OwnerLabEngine, map_provider_execution};

const VOICE_SAMPLE_RATE_HZ: u32 = 16_000;
const VOICE_CHANNELS: u16 = 1;
const MAX_VOICE_MILLIS: u64 = 30_000;
const OWNER_LAB_PROMPT_PREFIX: &str = "RT0 Owner Lab voice conversation. Answer the user's latest utterance briefly in Russian. Do not claim personal facts, opinions, memories, preferences, or private knowledge of the owner. If asked what the owner thinks, knows, remembers, or prefers, say that verified owner data is not available in this Owner Lab. User utterance: ";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabVoiceUsage {
    pub input_units: Option<u64>,
    pub output_units: Option<u64>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabVoiceResult {
    pub transcript: String,
    pub reply: String,
    pub locale: String,
    pub stt_millis: u64,
    pub llm_millis: u64,
    pub avatar_millis: u64,
    pub total_millis: u64,
    pub stt_usage: LabVoiceUsage,
    pub llm_usage: LabVoiceUsage,
}

impl OwnerLabEngine {
    #[must_use]
    pub fn with_voice(mut self, stt: Box<dyn SttPort>, llm: Box<dyn LlmPort>) -> Self {
        self.stt = Some(stt);
        self.llm = Some(llm);
        self
    }

    /// Runs one microphone utterance through STT -> LLM -> realtime avatar on one canonical turn.
    /// The callback receives a narrow interrupt-only capability before external provider work starts.
    ///
    /// # Errors
    /// Fails closed when voice providers are not configured, audio is malformed, session state is
    /// not active, authority changes, cancellation occurs, or any provider fails.
    pub fn voice_turn(
        &mut self,
        pcm_s16le_mono_16khz: Vec<u8>,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
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

        let llm_started = Instant::now();
        let mut generated = GeneratedTextBuffer::default();
        let llm_usage = turn
            .execute_llm(
                llm.as_ref(),
                &LlmRequest {
                    locale: transcript.locale.clone(),
                    context: format!("{OWNER_LAB_PROMPT_PREFIX}{}", transcript.text),
                },
                &mut generated,
            )
            .map_err(|error| terminalize_provider_error(&turn, error))?;
        let llm_millis = elapsed_millis(llm_started);
        let reply = generated.into_string();
        if reply.trim().is_empty() {
            return Err(terminalize_failed_turn(&turn, LabError::InvalidInput));
        }

        turn.begin_output().map_err(LabError::Runtime)?;
        let avatar_started = Instant::now();
        turn.speak_realtime_avatar_text(self.provider.as_ref(), handle, &reply)
            .map_err(|error| terminalize_provider_error(&turn, error))?;
        let avatar_millis = elapsed_millis(avatar_started);
        turn.complete().map_err(LabError::Runtime)?;

        Ok(LabVoiceResult {
            transcript: transcript.text,
            reply,
            locale: transcript.locale,
            stt_millis,
            llm_millis,
            avatar_millis,
            total_millis: elapsed_millis(total_started),
            stt_usage: map_usage(&stt_usage),
            llm_usage: map_usage(&llm_usage),
        })
    }
}

fn terminalize_provider_error(turn: &ActiveTurn, error: ProviderExecutionError) -> LabError {
    let mapped = map_provider_execution(error);
    terminalize_failed_turn(turn, mapped)
}

fn terminalize_failed_turn(turn: &ActiveTurn, error: LabError) -> LabError {
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

fn map_usage(usage: &UsageEvidence) -> LabVoiceUsage {
    LabVoiceUsage {
        input_units: usage.input_units,
        output_units: usage.output_units,
        estimated_cost_microunits: usage.estimated_cost_microunits,
        provider_charge_microunits: usage.provider_charge_microunits,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
