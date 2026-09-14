use serde::Serialize;
use vpr_domain::{
    ClaimId, ClaimKind, CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaProfile,
    PersonaVersion, RealtimeSessionState, Rt0ReasonCode, SessionId, TurnId,
};
use vpr_integration::{
    LlmPort, RealtimeAvatarCapability, RealtimeAvatarPort, SttPort, WebRtcIceCandidate,
    WebRtcIceServer, WebRtcSessionDescription,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{
    ActiveSession, ActiveTurn, ProviderExecutionError, RealtimeAvatarHandle, SessionSecurityConfig,
};

use crate::owner_context::{OwnerContextError, ReviewedOwnerContext};

const PROVIDER_SCOPE: &str = "provider.egress";
const PERSONA_ID: &str = "rt0-owner-lab-persona";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerLabStartRequest {
    pub consent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLabTurnInput {
    Answer(WebRtcSessionDescription),
    Ice(WebRtcIceCandidate),
    Text(String),
    AudioUrl(String),
    Interrupt,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabSignalBundle {
    pub evidence_session_sequence: u64,
    pub offer: LabSessionDescription,
    pub ice_servers: Vec<LabIceServer>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabSessionDescription {
    pub kind: String,
    pub sdp: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnerContextState {
    Missing,
    Reviewed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabStatus {
    pub session_state: String,
    pub avatar_open: bool,
    pub egress_enabled: bool,
    pub voice_ready: bool,
    pub owner_context_state: OwnerContextState,
    pub persona_version: u64,
    pub reviewed_owner_claims: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabError {
    EgressDisabled,
    ConsentRequired,
    InvalidInput,
    InvalidState,
    Runtime(Rt0ReasonCode),
    Provider(Rt0ReasonCode),
    Internal,
}

impl LabError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EgressDisabled => "EGRESS_DENIED",
            Self::ConsentRequired => "CONSENT_REQUIRED",
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidState => "INVALID_STATE_TRANSITION",
            Self::Runtime(reason) | Self::Provider(reason) => reason.as_str(),
            Self::Internal => "INTERNAL_ERROR",
        }
    }
}

pub struct OwnerLabEngine {
    persona: PersonaIdentity,
    reviewed_owner_context: Option<ReviewedOwnerContext>,
    provider: Box<dyn RealtimeAvatarPort>,
    stt: Option<Box<dyn SttPort>>,
    llm: Option<Box<dyn LlmPort>>,
    session: Option<ActiveSession>,
    avatar: Option<RealtimeAvatarHandle>,
    session_counter: u64,
    turn_counter: u64,
    egress_enabled: bool,
}

impl OwnerLabEngine {
    /// Creates the local owner-lab orchestration boundary around one realtime-avatar provider.
    ///
    /// # Errors
    /// Returns an internal error if the fixed RT0 persona identity cannot be constructed.
    pub fn new(
        provider: Box<dyn RealtimeAvatarPort>,
        egress_enabled: bool,
    ) -> Result<Self, LabError> {
        let persona_id = PersonaId::new(PERSONA_ID).map_err(|_| LabError::Internal)?;
        let version = PersonaVersion::new(1).ok_or(LabError::Internal)?;
        Ok(Self {
            persona: PersonaIdentity::new(persona_id, version, PersonaMode::DigitalTwin),
            reviewed_owner_context: None,
            provider,
            stt: None,
            llm: None,
            session: None,
            avatar: None,
            session_counter: 0,
            turn_counter: 0,
            egress_enabled,
        })
    }

    /// Binds an explicitly reviewed `DIGITAL_TWIN` profile as the canonical owner context for
    /// subsequent turns. The profile's Persona identity/version becomes the turn snapshot source.
    ///
    /// # Errors
    /// Fails closed if capture/review is incomplete or the profile is not a `DIGITAL_TWIN`.
    pub fn with_reviewed_profile(mut self, profile: PersonaProfile) -> Result<Self, LabError> {
        self.bind_reviewed_profile(profile)?;
        Ok(self)
    }

    /// Replaces the canonical reviewed owner context between realtime sessions.
    ///
    /// # Errors
    /// Fails closed while a realtime session is active, or when the supplied profile has not
    /// completed explicit `DIGITAL_TWIN` review.
    pub fn bind_reviewed_profile(&mut self, profile: PersonaProfile) -> Result<(), LabError> {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.state() != RealtimeSessionState::Closed)
        {
            return Err(LabError::InvalidState);
        }
        let context = ReviewedOwnerContext::new(profile).map_err(map_owner_context_error)?;
        self.reviewed_owner_context = Some(context);
        Ok(())
    }

    /// Corrects one owner-reviewed claim. The domain profile preserves the previous revision and
    /// advances `PersonaVersion`; the next turn snapshots the corrected version.
    ///
    /// # Errors
    /// Fails closed when no reviewed owner context is bound or the correction is rejected.
    pub fn correct_owner_claim(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), LabError> {
        self.reviewed_owner_context
            .as_mut()
            .ok_or(LabError::InvalidState)?
            .correct_claim(id, statement, kind)
            .map_err(map_owner_context_error)
    }

    #[must_use]
    pub fn status(&self) -> LabStatus {
        LabStatus {
            session_state: self.session.as_ref().map_or_else(
                || "none".to_owned(),
                |session| state_name(session.state()).to_owned(),
            ),
            avatar_open: self
                .avatar
                .as_ref()
                .is_some_and(|handle| !handle.is_closed()),
            egress_enabled: self.egress_enabled,
            voice_ready: self.stt.is_some() && self.llm.is_some(),
            owner_context_state: if self.reviewed_owner_context.is_some() {
                OwnerContextState::Reviewed
            } else {
                OwnerContextState::Missing
            },
            persona_version: self.persona_identity().version().get(),
            reviewed_owner_claims: self
                .reviewed_owner_context
                .as_ref()
                .map_or(0, ReviewedOwnerContext::claim_count),
        }
    }

    /// Starts one owner-lab runtime session and opens the provider realtime-avatar session.
    ///
    /// # Errors
    /// Fails closed without both process-level egress enablement and explicit user consent.
    pub fn start(&mut self, request: OwnerLabStartRequest) -> Result<LabSignalBundle, LabError> {
        if !self.egress_enabled {
            return Err(LabError::EgressDisabled);
        }
        if !request.consent {
            return Err(LabError::ConsentRequired);
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.state() != RealtimeSessionState::Closed)
        {
            return Err(LabError::InvalidState);
        }

        self.session_counter = self
            .session_counter
            .checked_add(1)
            .ok_or(LabError::Internal)?;
        let session_id = SessionId::new(format!("owner-lab-session-{}", self.session_counter))
            .map_err(|_| LabError::Internal)?;
        let authority = provider_authority()?;
        let mut session = ActiveSession::new(
            session_id,
            self.persona_identity().id().clone(),
            SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
        );
        session.activate().map_err(LabError::Runtime)?;
        let turn = self.new_turn_for(&session)?;
        let handle = turn
            .open_realtime_avatar(self.provider.as_ref())
            .map_err(map_provider_execution)?;
        let bundle = signal_bundle(self.provider.as_ref(), &handle, self.session_counter);
        self.session = Some(session);
        self.avatar = Some(handle);
        Ok(bundle)
    }

    /// Applies one browser signaling/content operation through a fresh canonical turn.
    ///
    /// # Errors
    /// Returns a stable fail-closed reason if session/authority/provider state is not current.
    pub fn apply(&mut self, input: OwnerLabTurnInput) -> Result<(), LabError> {
        let turn = self.new_turn()?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        match input {
            OwnerLabTurnInput::Answer(answer) => {
                if answer.kind != "answer" || answer.sdp.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                turn.submit_realtime_avatar_answer(self.provider.as_ref(), handle, &answer)
            }
            OwnerLabTurnInput::Ice(candidate) => {
                turn.submit_realtime_avatar_ice(self.provider.as_ref(), handle, &candidate)
            }
            OwnerLabTurnInput::Text(text) => {
                if text.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                turn.speak_realtime_avatar_text(self.provider.as_ref(), handle, &text)
            }
            OwnerLabTurnInput::AudioUrl(url) => {
                if url.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                turn.speak_realtime_avatar_audio_url(self.provider.as_ref(), handle, &url)
            }
            OwnerLabTurnInput::Interrupt => {
                turn.interrupt_realtime_avatar(self.provider.as_ref(), handle)
            }
        }
        .map_err(map_provider_execution)
    }

    /// Revokes canonical authority first, then best-effort closes the remote avatar resource.
    ///
    /// # Errors
    /// Returns a stable runtime/provider reason while preserving retryable cleanup state.
    pub fn revoke(&mut self) -> Result<(), LabError> {
        let session = self.session.as_mut().ok_or(LabError::InvalidState)?;
        match session.state() {
            RealtimeSessionState::Active => session.revoke().map_err(LabError::Runtime)?,
            RealtimeSessionState::Revoked => {}
            RealtimeSessionState::Created
            | RealtimeSessionState::Draining
            | RealtimeSessionState::Closed => return Err(LabError::InvalidState),
        }
        self.close_avatar_resource()
    }

    /// Closes the avatar resource and canonical runtime session.
    ///
    /// # Errors
    /// Returns a stable reason and never marks provider cleanup complete unless confirmed.
    pub fn close(&mut self) -> Result<(), LabError> {
        self.close_avatar_resource()?;
        let session = self.session.as_mut().ok_or(LabError::InvalidState)?;
        match session.state() {
            RealtimeSessionState::Active => {
                session.begin_draining().map_err(LabError::Runtime)?;
                session.close().map_err(LabError::Runtime)?;
            }
            RealtimeSessionState::Draining | RealtimeSessionState::Revoked => {
                session.close().map_err(LabError::Runtime)?;
            }
            RealtimeSessionState::Closed => {}
            RealtimeSessionState::Created => return Err(LabError::InvalidState),
        }
        Ok(())
    }

    fn close_avatar_resource(&mut self) -> Result<(), LabError> {
        let Some(handle) = self.avatar.as_mut() else {
            return Ok(());
        };
        let session = self.session.as_ref().ok_or(LabError::InvalidState)?;
        session
            .close_realtime_avatar(self.provider.as_ref(), handle)
            .map_err(map_provider_execution)?;
        self.avatar = None;
        Ok(())
    }

    fn persona_identity(&self) -> &PersonaIdentity {
        self.reviewed_owner_context
            .as_ref()
            .map_or(&self.persona, ReviewedOwnerContext::identity)
    }

    fn new_turn(&mut self) -> Result<ActiveTurn, LabError> {
        let session = self.session.take().ok_or(LabError::InvalidState)?;
        let result = self.new_turn_for(&session);
        self.session = Some(session);
        result
    }

    fn new_turn_for(&mut self, session: &ActiveSession) -> Result<ActiveTurn, LabError> {
        self.turn_counter = self.turn_counter.checked_add(1).ok_or(LabError::Internal)?;
        let turn = ActiveTurn::new(
            TurnId::new(format!("owner-lab-turn-{}", self.turn_counter))
                .map_err(|_| LabError::Internal)?,
            CorrelationId::new(format!("owner-lab-correlation-{}", self.turn_counter))
                .map_err(|_| LabError::Internal)?,
            self.persona_identity(),
            session,
        )
        .map_err(|reason| LabError::Runtime(reason.reason_code()))?;
        turn.authorize().map_err(LabError::Runtime)?;
        turn.begin_processing().map_err(LabError::Runtime)?;
        Ok(turn)
    }
}

fn signal_bundle(
    port: &dyn RealtimeAvatarPort,
    handle: &RealtimeAvatarHandle,
    evidence_session_sequence: u64,
) -> LabSignalBundle {
    let provider_capabilities = port.capabilities();
    let capabilities = [
        (RealtimeAvatarCapability::TextInput, "text"),
        (RealtimeAvatarCapability::AudioUrlInput, "audio_url"),
        (RealtimeAvatarCapability::Interrupt, "interrupt"),
    ]
    .into_iter()
    .filter(|(capability, _)| provider_capabilities.supports(*capability))
    .map(|(_, name)| name.to_owned())
    .collect();
    LabSignalBundle {
        evidence_session_sequence,
        offer: LabSessionDescription {
            kind: handle.offer().kind.clone(),
            sdp: handle.offer().sdp.clone(),
        },
        ice_servers: handle.ice_servers().iter().map(map_ice_server).collect(),
        capabilities,
    }
}

fn map_ice_server(server: &WebRtcIceServer) -> LabIceServer {
    LabIceServer {
        urls: server.urls.clone(),
        username: server.username.clone(),
        credential: server.credential.clone(),
    }
}

fn provider_authority() -> Result<EffectiveAuthority, LabError> {
    let scope = AuthorityScope::new(PROVIDER_SCOPE).ok_or(LabError::Internal)?;
    Ok(EffectiveAuthority::compose(&[AuthorityLayer::new(
        [scope],
        [],
    )]))
}

fn map_provider_execution(error: ProviderExecutionError) -> LabError {
    match error {
        ProviderExecutionError::Denied(reason) => LabError::Runtime(reason.reason_code()),
        ProviderExecutionError::Provider(provider) => {
            LabError::Provider(vpr_runtime::provider_reason_code(provider.kind))
        }
    }
}

const fn map_owner_context_error(error: OwnerContextError) -> LabError {
    match error {
        OwnerContextError::ProfileNotReviewed => LabError::InvalidState,
        OwnerContextError::CorrectionRejected => LabError::InvalidInput,
    }
}

const fn state_name(state: RealtimeSessionState) -> &'static str {
    match state {
        RealtimeSessionState::Created => "created",
        RealtimeSessionState::Active => "active",
        RealtimeSessionState::Draining => "draining",
        RealtimeSessionState::Revoked => "revoked",
        RealtimeSessionState::Closed => "closed",
    }
}

mod voice;
pub use voice::{LabVoiceResult, LabVoiceUsage};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod voice_tests;
