use std::time::Instant;

use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, Rt0ReasonCode,
    SessionId, TurnId,
};
use vpr_evaluation::{
    AvatarProbeEvidence, LiveProviderProbeReceipt, LlmProbeEvidence, ProbeUsage,
    RT0_LIVE_PROVIDER_PROBE_SCHEMA, SttProbeEvidence, TtsProbeEvidence, sha256_hex,
};
use vpr_integration::{
    AudioInput, GeneratedAudioBuffer, GeneratedTextBuffer, LlmRequest, PcmSampleFormat,
    RealtimeAvatarPort, SttRequest, TtsRequest, UsageEvidence, UsageUnit,
};
use vpr_owner_lab::{
    LabError, OwnerLabEngine, OwnerLabStartRequest, PreparedTtsProbe, build_tts_probe_from_env,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{ActiveSession, ActiveTurn, SessionSecurityConfig};

use crate::PreparedLiveProof;

const MAX_AUDIO_MILLIS: u64 = 30_000;
const SAMPLE_RATE_HZ: u32 = 16_000;
const CHANNELS: u16 = 1;
const PROVIDER_SCOPE: &str = "provider.egress";
const LLM_PROBE_PROMPT: &str = "Ответь одним коротким словом на русском языке: готов.";
const TTS_PROBE_TEXT: &str = "Готов.";
const MAX_PCM_BYTES: usize = 960_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveProviderProbeError {
    InvalidAudio,
    Runtime(Rt0ReasonCode),
    Stt(Rt0ReasonCode),
    InvalidSttOutput,
    Llm(Rt0ReasonCode),
    InvalidLlmOutput,
    TtsConfiguration,
    Tts(Rt0ReasonCode),
    InvalidTtsOutput,
    Avatar(Rt0ReasonCode),
    AvatarCleanup(Rt0ReasonCode),
    Internal,
}

impl LiveProviderProbeError {
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        match self {
            Self::InvalidAudio => "input",
            Self::Runtime(_) | Self::Internal => "runtime",
            Self::Stt(_) | Self::InvalidSttOutput => "stt",
            Self::Llm(_) | Self::InvalidLlmOutput => "llm",
            Self::TtsConfiguration | Self::Tts(_) | Self::InvalidTtsOutput => "tts",
            Self::Avatar(_) => "avatar_open",
            Self::AvatarCleanup(_) => "avatar_cleanup",
        }
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidAudio => "INVALID_INPUT",
            Self::Runtime(reason)
            | Self::Stt(reason)
            | Self::Llm(reason)
            | Self::Tts(reason)
            | Self::Avatar(reason)
            | Self::AvatarCleanup(reason) => reason.as_str(),
            Self::TtsConfiguration => "PROVIDER_CONFIGURATION_INVALID",
            Self::InvalidSttOutput | Self::InvalidLlmOutput | Self::InvalidTtsOutput => {
                "PROVIDER_INVALID_RESPONSE"
            }
            Self::Internal => "INTERNAL_ERROR",
        }
    }
}

/// Probes credentialed STT, LLM, TTS, and realtime-avatar control-plane reachability through canonical runtime paths.
///
/// The returned receipt intentionally contains no transcript, generated reply, raw audio, WebRTC signaling,
/// provider session identifiers, or credentials. It is reachability evidence only, not conversation evidence.
///
/// # Errors
/// Returns a stable stage/reason when audio is malformed, canonical runtime authorization fails, a provider
/// operation fails, provider output is invalid, or avatar cleanup cannot be confirmed.
pub fn run_provider_probe(
    prepared: PreparedLiveProof,
    pcm_s16le_mono_16khz: Vec<u8>,
) -> Result<LiveProviderProbeReceipt, LiveProviderProbeError> {
    if pcm_s16le_mono_16khz.len() < 32
        || pcm_s16le_mono_16khz.len() % 2 != 0
        || pcm_s16le_mono_16khz.len() > MAX_PCM_BYTES
    {
        return Err(LiveProviderProbeError::InvalidAudio);
    }
    let tts = build_tts_probe_from_env().map_err(|_| LiveProviderProbeError::TtsConfiguration)?;
    run_provider_probe_with_tts(prepared, pcm_s16le_mono_16khz, tts)
}

pub(crate) fn run_provider_probe_with_tts(
    prepared: PreparedLiveProof,
    pcm_s16le_mono_16khz: Vec<u8>,
    tts: PreparedTtsProbe,
) -> Result<LiveProviderProbeReceipt, LiveProviderProbeError> {
    let input_audio_sha256 = sha256_hex(&pcm_s16le_mono_16khz);
    let audio = AudioInput {
        pcm: pcm_s16le_mono_16khz,
        sample_rate_hz: SAMPLE_RATE_HZ,
        channels: CHANNELS,
        sample_format: PcmSampleFormat::S16Le,
    };
    let duration = audio
        .duration_millis()
        .ok_or(LiveProviderProbeError::InvalidAudio)?;
    if duration == 0 || duration > MAX_AUDIO_MILLIS {
        return Err(LiveProviderProbeError::InvalidAudio);
    }

    let (receipt, mut providers) = prepared.into_parts();
    let stt = providers
        .stt
        .take()
        .ok_or(LiveProviderProbeError::Internal)?;
    let llm = providers
        .llm
        .take()
        .ok_or(LiveProviderProbeError::Internal)?;
    let turn = canonical_turn()?;

    let stt_started = Instant::now();
    let (transcript, stt_usage) = turn
        .execute_stt(
            stt.as_ref(),
            &SttRequest {
                audio,
                locale_hint: Some("ru-RU".into()),
            },
        )
        .map_err(|error| {
            terminalize_failed_probe_turn(&turn, LiveProviderProbeError::Stt(error.reason_code()))
        })?;
    let stt_millis = elapsed_millis(stt_started);
    if transcript.text.trim().is_empty() {
        return Err(terminalize_failed_probe_turn(
            &turn,
            LiveProviderProbeError::InvalidSttOutput,
        ));
    }
    let transcript_chars = count_chars(&transcript.text);

    let llm_started = Instant::now();
    let mut generated = GeneratedTextBuffer::default();
    let llm_usage = turn
        .execute_llm(
            llm.as_ref(),
            &LlmRequest {
                locale: "ru-RU".into(),
                context: LLM_PROBE_PROMPT.into(),
            },
            &mut generated,
        )
        .map_err(|error| {
            terminalize_failed_probe_turn(&turn, LiveProviderProbeError::Llm(error.reason_code()))
        })?;
    let llm_millis = elapsed_millis(llm_started);
    if generated.as_str().trim().is_empty() {
        return Err(terminalize_failed_probe_turn(
            &turn,
            LiveProviderProbeError::InvalidLlmOutput,
        ));
    }
    let output_chars = count_chars(generated.as_str());

    let tts_evidence = run_tts_probe(&turn, tts)?;

    turn.begin_output()
        .map_err(LiveProviderProbeError::Runtime)?;
    turn.complete().map_err(LiveProviderProbeError::Runtime)?;

    let avatar_evidence = run_avatar_probe(providers.avatar)?;

    Ok(LiveProviderProbeReceipt {
        schema_version: RT0_LIVE_PROVIDER_PROBE_SCHEMA.into(),
        candidate_sha: receipt.candidate_sha,
        provider_state_sha256: receipt.provider_state_sha256,
        input_audio_sha256,
        input_audio_millis: duration,
        scope: "credentialed_provider_reachability_only".into(),
        conversation_evidence: false,
        output_delivery_proven: false,
        stt: SttProbeEvidence {
            latency_millis: stt_millis,
            transcript_chars,
            usage: map_usage(&stt_usage),
        },
        llm: LlmProbeEvidence {
            latency_millis: llm_millis,
            output_chars,
            usage: map_usage(&llm_usage),
        },
        tts: tts_evidence,
        avatar: avatar_evidence,
    })
}

fn run_tts_probe(
    turn: &ActiveTurn,
    tts: PreparedTtsProbe,
) -> Result<TtsProbeEvidence, LiveProviderProbeError> {
    let started = Instant::now();
    let mut synthesized = GeneratedAudioBuffer::default();
    let usage = turn
        .execute_tts(
            tts.provider.as_ref(),
            &TtsRequest {
                text: TTS_PROBE_TEXT.into(),
                locale_hint: Some("ru-RU".into()),
            },
            &mut synthesized,
        )
        .map_err(|error| {
            terminalize_failed_probe_turn(turn, LiveProviderProbeError::Tts(error.reason_code()))
        })?;
    let audio_millis = synthesized.duration_millis().ok_or_else(|| {
        terminalize_failed_probe_turn(turn, LiveProviderProbeError::InvalidTtsOutput)
    })?;
    if synthesized.pcm().is_empty() || audio_millis == 0 || audio_millis > MAX_AUDIO_MILLIS {
        return Err(terminalize_failed_probe_turn(
            turn,
            LiveProviderProbeError::InvalidTtsOutput,
        ));
    }
    Ok(TtsProbeEvidence {
        provider: tts.descriptor.provider,
        model_or_representation: tts.descriptor.model_or_representation,
        configuration_fingerprint_sha256: tts.descriptor.configuration_fingerprint_sha256,
        latency_millis: elapsed_millis(started),
        audio_sha256: sha256_hex(synthesized.pcm()),
        audio_millis,
        usage: map_usage(&usage),
    })
}

fn run_avatar_probe(
    provider: Box<dyn RealtimeAvatarPort>,
) -> Result<AvatarProbeEvidence, LiveProviderProbeError> {
    let mut avatar = OwnerLabEngine::new(provider, true)
        .map_err(|error| LiveProviderProbeError::Avatar(lab_reason(&error)))?;
    let avatar_started = Instant::now();
    avatar
        .start(OwnerLabStartRequest { consent: true })
        .map_err(|error| LiveProviderProbeError::Avatar(lab_reason(&error)))?;
    let open_millis = elapsed_millis(avatar_started);
    let close_started = Instant::now();
    if let Err(error) = avatar.close() {
        let reason = lab_reason(&error);
        let _ = avatar.revoke();
        return Err(LiveProviderProbeError::AvatarCleanup(reason));
    }
    Ok(AvatarProbeEvidence {
        open_millis,
        close_millis: elapsed_millis(close_started),
    })
}

fn terminalize_failed_probe_turn(
    turn: &ActiveTurn,
    error: LiveProviderProbeError,
) -> LiveProviderProbeError {
    let _ = turn.fail();
    error
}

fn canonical_turn() -> Result<ActiveTurn, LiveProviderProbeError> {
    let persona_id = PersonaId::new("rt0-live-provider-probe-persona")
        .map_err(|_| LiveProviderProbeError::Internal)?;
    let version = PersonaVersion::new(1).ok_or(LiveProviderProbeError::Internal)?;
    let persona = PersonaIdentity::new(persona_id, version, PersonaMode::DigitalTwin);
    let scope = AuthorityScope::new(PROVIDER_SCOPE).ok_or(LiveProviderProbeError::Internal)?;
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("rt0-live-provider-probe-session")
            .map_err(|_| LiveProviderProbeError::Internal)?,
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session
        .activate()
        .map_err(LiveProviderProbeError::Runtime)?;
    let turn = ActiveTurn::new(
        TurnId::new("rt0-live-provider-probe-turn")
            .map_err(|_| LiveProviderProbeError::Internal)?,
        CorrelationId::new("rt0-live-provider-probe-correlation")
            .map_err(|_| LiveProviderProbeError::Internal)?,
        &persona,
        &session,
    )
    .map_err(|reason| LiveProviderProbeError::Runtime(reason.reason_code()))?;
    turn.authorize().map_err(LiveProviderProbeError::Runtime)?;
    turn.begin_processing()
        .map_err(LiveProviderProbeError::Runtime)?;
    Ok(turn)
}

fn lab_reason(error: &LabError) -> Rt0ReasonCode {
    match error {
        LabError::EgressDisabled => Rt0ReasonCode::EgressDenied,
        LabError::ConsentRequired => Rt0ReasonCode::ConsentRequired,
        LabError::InvalidInput | LabError::InvalidState => Rt0ReasonCode::InvalidStateTransition,
        LabError::Runtime(reason) | LabError::Provider(reason) => *reason,
        LabError::Internal => Rt0ReasonCode::InternalError,
    }
}

fn map_usage(usage: &UsageEvidence) -> ProbeUsage {
    ProbeUsage {
        input_units: usage.input_units,
        input_unit: usage.input_unit.map(usage_unit_name).map(str::to_owned),
        output_units: usage.output_units,
        output_unit: usage.output_unit.map(usage_unit_name).map(str::to_owned),
        estimated_cost_microunits: usage.estimated_cost_microunits,
        provider_charge_microunits: usage.provider_charge_microunits,
    }
}

const fn usage_unit_name(unit: UsageUnit) -> &'static str {
    match unit {
        UsageUnit::Token => "token",
        UsageUnit::TextCharacter => "text_character",
        UsageUnit::AudioMillisecond => "audio_millisecond",
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn count_chars(value: &str) -> u64 {
    u64::try_from(value.chars().count()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
