use serde::{Deserialize, Serialize};
use vpr_domain::{
    ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord,
    PersonaId, PersonaIdentity, PersonaMode, PersonaProfile, PersonaVersion, SourceKind,
    VerificationState,
};
use vpr_evaluation::sha256_hex;
use vpr_owner_lab::{LabSessionAudience, LabVoiceResult, OwnerLabEngine, OwnerLabStartRequest};

use crate::PreparedLiveProof;

pub const RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA: &str = "rt0-live-conversation-attempt-0.1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConversationProfileInput {
    pub schema_version: String,
    pub persona_id: String,
    pub owner_review_confirmed: bool,
    pub claims: Vec<LiveConversationClaimInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConversationClaimInput {
    pub claim_id: String,
    pub statement: String,
    pub kind: String,
    pub owner_approved: bool,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LiveConversationTurnReceipt {
    pub audience: LabSessionAudience,
    pub input_audio_sha256: String,
    pub transcript_sha256: String,
    pub transcript_chars: u64,
    pub reply_sha256: String,
    pub reply_chars: u64,
    pub locale: String,
    pub stt_millis: u64,
    pub llm_millis: u64,
    pub avatar_submit_millis: u64,
    pub total_millis: u64,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Proven,
    NotProven,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LiveConversationAttemptReceipt {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub profile_input_sha256: String,
    pub persona_id_sha256: String,
    pub persona_version: u64,
    pub reviewed_claims: usize,
    pub owner: LiveConversationTurnReceipt,
    pub visitor: LiveConversationTurnReceipt,
    pub conversation_attempted: bool,
    pub provider_output_submitted: bool,
    pub browser_media_playback: ProofStatus,
    pub video_render: ProofStatus,
    pub human_review: ProofStatus,
}
pub const RT0_LIVE_CONVERSATION_PROFILE_SCHEMA: &str = "rt0-live-conversation-profile-0.1";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LiveConversationAttemptError {
    InvalidProfile,
    InvalidAudio,
    VoiceProvidersUnavailable,
    OwnerSessionFailed,
    OwnerTurnFailed,
    OwnerCleanupFailed,
    VisitorSessionFailed,
    VisitorTurnFailed,
    VisitorCleanupFailed,
    Internal,
}

impl LiveConversationAttemptError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidProfile => "INVALID_PROFILE",
            Self::InvalidAudio => "INVALID_INPUT",
            Self::VoiceProvidersUnavailable => "PROVIDER_CONFIGURATION_INVALID",
            Self::OwnerSessionFailed
            | Self::OwnerTurnFailed
            | Self::OwnerCleanupFailed
            | Self::VisitorSessionFailed
            | Self::VisitorTurnFailed
            | Self::VisitorCleanupFailed => "LIVE_CONVERSATION_FAILED",
            Self::Internal => "INTERNAL_ERROR",
        }
    }

    #[must_use]
    pub const fn stage(self) -> &'static str {
        match self {
            Self::InvalidProfile => "profile",
            Self::InvalidAudio => "input",
            Self::VoiceProvidersUnavailable => "providers",
            Self::OwnerSessionFailed => "owner_session",
            Self::OwnerTurnFailed => "owner_turn",
            Self::OwnerCleanupFailed => "owner_cleanup",
            Self::VisitorSessionFailed => "visitor_session",
            Self::VisitorTurnFailed => "visitor_turn",
            Self::VisitorCleanupFailed => "visitor_cleanup",
            Self::Internal => "runtime",
        }
    }
}

/// Validates private profile and owner/visitor PCM inputs without constructing or calling providers.
///
/// This is used by the atomic live-candidate CLI path so malformed private inputs fail before
/// any credentialed egress or provider charge can occur.
///
/// # Errors
/// Returns the same input/profile validation errors used by the live conversation attempt.
pub fn validate_live_conversation_inputs(
    profile_input_bytes: &[u8],
    owner_audio: &[u8],
    visitor_audio: &[u8],
) -> Result<(), LiveConversationAttemptError> {
    validate_audio(owner_audio)?;
    validate_audio(visitor_audio)?;
    let input: LiveConversationProfileInput = serde_json::from_slice(profile_input_bytes)
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
    reviewed_profile(&input)?;
    Ok(())
}

/// Runs one credentialed owner turn followed by one visitor-scoped turn through the canonical
/// Owner Lab engine and returns only sanitized attempt evidence.
///
/// # Errors
/// Fails closed for invalid or unreviewed profile input, malformed audio, unavailable voice
/// providers, owner/visitor session or turn failures, or unconfirmed avatar cleanup.
pub fn run_live_conversation_attempt(
    prepared: PreparedLiveProof,
    profile_input_bytes: &[u8],
    owner_audio: Vec<u8>,
    visitor_audio: Vec<u8>,
) -> Result<LiveConversationAttemptReceipt, LiveConversationAttemptError> {
    validate_audio(&owner_audio)?;
    validate_audio(&visitor_audio)?;
    let input: LiveConversationProfileInput = serde_json::from_slice(profile_input_bytes)
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
    let profile = reviewed_profile(&input)?;
    let persona_id_sha256 = sha256_hex(input.persona_id.as_bytes());
    let persona_version = profile.identity().version().get();
    let reviewed_claims = profile.claims().len();
    let profile_input_sha256 = sha256_hex(profile_input_bytes);
    let owner_audio_sha256 = sha256_hex(&owner_audio);
    let visitor_audio_sha256 = sha256_hex(&visitor_audio);
    let (preflight, mut providers) = prepared.into_parts();
    let stt = providers
        .stt
        .take()
        .ok_or(LiveConversationAttemptError::VoiceProvidersUnavailable)?;
    let llm = providers
        .llm
        .take()
        .ok_or(LiveConversationAttemptError::VoiceProvidersUnavailable)?;
    let mut engine = OwnerLabEngine::new(providers.avatar, true)
        .map_err(|_| LiveConversationAttemptError::Internal)?
        .with_reviewed_profile(profile)
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?
        .with_voice(stt, llm);

    engine
        .start(OwnerLabStartRequest { consent: true })
        .map_err(|_| LiveConversationAttemptError::OwnerSessionFailed)?;
    let Ok(owner_result) = engine.voice_turn(owner_audio, |_| {}) else {
        let _ = engine.revoke();
        let _ = engine.close();
        return Err(LiveConversationAttemptError::OwnerTurnFailed);
    };
    close_or_cleanup(
        &mut engine,
        LiveConversationAttemptError::OwnerCleanupFailed,
    )?;
    engine
        .start_visitor(OwnerLabStartRequest { consent: true })
        .map_err(|_| LiveConversationAttemptError::VisitorSessionFailed)?;
    let Ok(visitor_result) = engine.voice_turn(visitor_audio, |_| {}) else {
        let _ = engine.revoke();
        let _ = engine.close();
        return Err(LiveConversationAttemptError::VisitorTurnFailed);
    };
    close_or_cleanup(
        &mut engine,
        LiveConversationAttemptError::VisitorCleanupFailed,
    )?;

    Ok(LiveConversationAttemptReceipt {
        schema_version: RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA.into(),
        candidate_sha: preflight.candidate_sha,
        provider_state_sha256: preflight.provider_state_sha256,
        profile_input_sha256,
        persona_id_sha256,
        persona_version,
        reviewed_claims,
        owner: turn_receipt(LabSessionAudience::Owner, owner_audio_sha256, owner_result),
        visitor: turn_receipt(
            LabSessionAudience::Visitor,
            visitor_audio_sha256,
            visitor_result,
        ),
        conversation_attempted: true,
        provider_output_submitted: true,
        browser_media_playback: ProofStatus::NotProven,
        video_render: ProofStatus::NotProven,
        human_review: ProofStatus::NotProven,
    })
}

fn close_or_cleanup(
    engine: &mut OwnerLabEngine,
    failure: LiveConversationAttemptError,
) -> Result<(), LiveConversationAttemptError> {
    if engine.close().is_ok() {
        return Ok(());
    }
    let _ = engine.revoke();
    let _ = engine.close();
    Err(failure)
}

fn reviewed_profile(
    input: &LiveConversationProfileInput,
) -> Result<PersonaProfile, LiveConversationAttemptError> {
    if input.schema_version != RT0_LIVE_CONVERSATION_PROFILE_SCHEMA
        || !input.owner_review_confirmed
        || input.claims.is_empty()
        || input.claims.iter().any(|claim| !claim.owner_approved)
    {
        return Err(LiveConversationAttemptError::InvalidProfile);
    }
    let identity = PersonaIdentity::new(
        PersonaId::new(input.persona_id.clone())
            .map_err(|_| LiveConversationAttemptError::InvalidProfile)?,
        PersonaVersion::new(1).ok_or(LiveConversationAttemptError::InvalidProfile)?,
        PersonaMode::DigitalTwin,
    );
    let mut profile = PersonaProfile::new(identity, ConstitutionBoundary::strict_digital_twin());
    let mut ids = Vec::with_capacity(input.claims.len());
    for claim in &input.claims {
        let id = ClaimId::new(claim.claim_id.clone())
            .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
        let record = OwnerClaimRecord::capture(
            id.clone(),
            OwnerClaim {
                statement: claim.statement.clone(),
                kind: claim_kind(&claim.kind)?,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        )
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
        profile
            .add_captured_claim(record)
            .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
        ids.push(id);
    }
    profile
        .mark_capture_complete()
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
    for id in &ids {
        profile
            .approve_claim(id)
            .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
    }
    profile
        .approve_initial_review()
        .map_err(|_| LiveConversationAttemptError::InvalidProfile)?;
    Ok(profile)
}

fn claim_kind(value: &str) -> Result<ClaimKind, LiveConversationAttemptError> {
    match value {
        "factual" => Ok(ClaimKind::Factual),
        "opinion" => Ok(ClaimKind::Opinion),
        "preference" => Ok(ClaimKind::Preference),
        "prediction" => Ok(ClaimKind::Prediction),
        "value_judgment" => Ok(ClaimKind::ValueJudgment),
        _ => Err(LiveConversationAttemptError::InvalidProfile),
    }
}

fn validate_audio(bytes: &[u8]) -> Result<(), LiveConversationAttemptError> {
    const SAMPLE_RATE_HZ: u64 = 16_000;
    const BYTES_PER_SAMPLE: u64 = 2;
    const MAX_MILLIS: u64 = 30_000;
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return Err(LiveConversationAttemptError::InvalidAudio);
    }
    let samples =
        u64::try_from(bytes.len() / 2).map_err(|_| LiveConversationAttemptError::InvalidAudio)?;
    let duration = samples
        .checked_mul(1_000)
        .and_then(|value| value.checked_div(SAMPLE_RATE_HZ))
        .ok_or(LiveConversationAttemptError::InvalidAudio)?;
    let max_bytes = SAMPLE_RATE_HZ
        .checked_mul(BYTES_PER_SAMPLE)
        .and_then(|value| value.checked_mul(MAX_MILLIS))
        .and_then(|value| value.checked_div(1_000))
        .ok_or(LiveConversationAttemptError::InvalidAudio)?;
    if duration == 0
        || u64::try_from(bytes.len())
            .ok()
            .is_none_or(|len| len > max_bytes)
    {
        return Err(LiveConversationAttemptError::InvalidAudio);
    }
    Ok(())
}

fn turn_receipt(
    audience: LabSessionAudience,
    input_audio_sha256: String,
    result: LabVoiceResult,
) -> LiveConversationTurnReceipt {
    LiveConversationTurnReceipt {
        audience,
        input_audio_sha256,
        transcript_sha256: sha256_hex(result.transcript.as_bytes()),
        transcript_chars: count_chars(&result.transcript),
        reply_sha256: sha256_hex(result.reply.as_bytes()),
        reply_chars: count_chars(&result.reply),
        locale: result.locale,
        stt_millis: result.stt_millis,
        llm_millis: result.llm_millis,
        avatar_submit_millis: result.avatar_millis,
        total_millis: result.total_millis,
        estimated_cost_microunits: known_sum(
            result.stt_usage.estimated_cost_microunits,
            result.llm_usage.estimated_cost_microunits,
        ),
        provider_charge_microunits: known_sum(
            result.stt_usage.provider_charge_microunits,
            result.llm_usage.provider_charge_microunits,
        ),
    }
}

fn known_sum(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    left.zip(right)
        .and_then(|(left, right)| left.checked_add(right))
}

fn count_chars(value: &str) -> u64 {
    u64::try_from(value.chars().count()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
