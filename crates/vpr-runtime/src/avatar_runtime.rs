use std::fmt::{Debug, Formatter, Result as FmtResult};

use vpr_domain::{Rt0ReasonCode, SessionId};
use vpr_integration::{
    ProviderDescriptor, RealtimeAvatarClientCommand, RealtimeAvatarClientControl,
    RealtimeAvatarClientEvent, RealtimeAvatarPort, RealtimeAvatarSession, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription,
};

use crate::error::{ProviderExecutionError, RuntimeDenyReason};
use crate::provider::ProviderOperation;
use crate::{ActiveSession, ActiveTurn, OutputDeliveryHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeAvatarOutputError {
    Runtime(Rt0ReasonCode),
    Provider(ProviderExecutionError),
}

impl RealtimeAvatarOutputError {
    #[must_use]
    pub const fn reason_code(&self) -> Rt0ReasonCode {
        match self {
            Self::Runtime(reason) => *reason,
            Self::Provider(error) => error.reason_code(),
        }
    }
}

pub struct RealtimeAvatarHandle {
    session_id: SessionId,
    provider: ProviderDescriptor,
    provider_session: RealtimeAvatarSession,
    client_control: Option<RealtimeAvatarClientControl>,
    closed: bool,
}

impl Debug for RealtimeAvatarHandle {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("RealtimeAvatarHandle")
            .field("provider", &self.provider.provider)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl RealtimeAvatarHandle {
    #[must_use]
    pub fn offer(&self) -> &WebRtcSessionDescription {
        &self.provider_session.offer
    }

    #[must_use]
    pub fn ice_servers(&self) -> &[WebRtcIceServer] {
        &self.provider_session.ice_servers
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    #[must_use]
    pub fn client_control(&self) -> Option<&RealtimeAvatarClientControl> {
        self.client_control.as_ref()
    }
}

impl ActiveTurn {
    /// Opens one session-scoped realtime avatar resource through the canonical turn egress gate.
    ///
    /// # Errors
    /// Returns a runtime denial or typed provider failure. If cancellation races with a successful
    /// provider create, runtime performs best-effort remote cleanup before returning cancellation.
    pub fn open_realtime_avatar(
        &self,
        port: &dyn RealtimeAvatarPort,
    ) -> Result<RealtimeAvatarHandle, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Avatar)
            .map_err(ProviderExecutionError::from)?;
        let provider_session = port
            .create_session(&permit.cancellation)
            .map_err(ProviderExecutionError::from)?;
        if vpr_integration::CancellationProbe::is_cancelled(&permit.cancellation) {
            let _cleanup = port.close_session(&provider_session);
            return Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled,
            ));
        }
        let client_control = port.client_control(&provider_session);
        Ok(RealtimeAvatarHandle {
            session_id: self.session_id.clone(),
            provider: port.descriptor(),
            provider_session,
            client_control,
            closed: false,
        })
    }

    /// Submits the local WebRTC answer through the current turn authorization boundary.
    ///
    /// # Errors
    /// Returns a denial for cross-session/provider/closed handles or a typed provider failure.
    pub fn submit_realtime_avatar_answer(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        answer: &WebRtcSessionDescription,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let permit = self.avatar_permit()?;
        port.submit_answer(&handle.provider_session, answer, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Submits one ICE candidate through the current turn authorization boundary.
    ///
    /// # Errors
    /// Returns a denial for cross-session/provider/closed handles or a typed provider failure.
    pub fn submit_realtime_avatar_ice(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        candidate: &WebRtcIceCandidate,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let permit = self.avatar_permit()?;
        port.submit_ice_candidate(&handle.provider_session, candidate, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Sends text to an existing session-scoped avatar using this turn's current authorization.
    ///
    /// # Errors
    /// Returns a denial for stale/cancelled authority or an adapter failure.
    pub fn speak_realtime_avatar_text(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let permit = self.avatar_permit()?;
        port.speak_text(&handle.provider_session, text, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Sends avatar text while allocating the canonical output-delivery segment used for later
    /// participant playback acknowledgement. A successful provider submission records `Sent`;
    /// browser playback must still reconcile the returned handle to `Played`.
    ///
    /// # Errors
    /// Returns before allocating output evidence for stale/denied avatar handles. Provider failure
    /// after allocation leaves the generated segment unplayed so terminalization can freeze it.
    pub fn deliver_realtime_avatar_text(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<OutputDeliveryHandle, RealtimeAvatarOutputError> {
        self.validate_avatar_handle(port, handle)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let permit = self
            .avatar_permit()
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let segment_id = self
            .begin_output_segment()
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        let delivery = OutputDeliveryHandle::new(self.snapshot.turn_id().clone(), segment_id);
        port.speak_text(&handle.provider_session, text, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        self.acknowledge_output_sent(&delivery)
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        Ok(delivery)
    }

    /// Sends an HTTPS audio URL to an existing avatar using this turn's current authorization.
    ///
    /// # Errors
    /// Returns a denial for stale/cancelled authority or an adapter failure.
    pub fn speak_realtime_avatar_audio_url(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        audio_url: &str,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let permit = self.avatar_permit()?;
        port.speak_audio_url(&handle.provider_session, audio_url, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Normalizes one provider data-channel message against this exact avatar handle.
    ///
    /// # Errors
    /// Returns a denial for a stale/cross-session handle or a typed provider parse failure.
    pub fn parse_realtime_avatar_client_event(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        message: &str,
    ) -> Result<Option<RealtimeAvatarClientEvent>, ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        port.parse_client_event(&handle.provider_session, message)
            .map_err(ProviderExecutionError::from)
    }

    /// Authorizes and prepares a provider-specific browser data-channel interruption command.
    ///
    /// Provider protocol details remain inside the adapter. Runtime only releases the opaque
    /// command after validating the exact avatar handle and current authority/egress policy.
    ///
    /// # Errors
    /// Returns a denial for stale/cancelled authority or a typed provider failure when client-side
    /// interruption is unavailable or malformed.
    pub fn prepare_realtime_avatar_client_interrupt(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        playback_id: &str,
    ) -> Result<RealtimeAvatarClientCommand, ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        if !handle
            .client_control
            .as_ref()
            .is_some_and(|control| control.interrupt)
        {
            return Err(ProviderExecutionError::Provider(vpr_integration::ProviderError {
                kind: vpr_integration::ProviderErrorKind::Unavailable,
                retryable: false,
            }));
        }
        let permit = self.avatar_permit()?;
        port.prepare_client_interrupt(
            &handle.provider_session,
            playback_id,
            &permit.cancellation,
        )
        .map_err(ProviderExecutionError::from)
    }

    /// Requests provider-side interruption using this turn's current authorization.
    ///
    /// # Errors
    /// Returns a denial or the provider's typed unsupported/failure result.
    pub fn interrupt_realtime_avatar(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let permit = self.avatar_permit()?;
        port.interrupt(&handle.provider_session, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    fn avatar_permit(
        &self,
    ) -> Result<crate::provider::ProviderExecutionPermit, ProviderExecutionError> {
        self.issue_provider_operation(ProviderOperation::Avatar)
            .map_err(ProviderExecutionError::from)
    }

    fn validate_avatar_handle(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
    ) -> Result<(), ProviderExecutionError> {
        if handle.closed
            || handle.session_id != self.session_id
            || handle.provider != port.descriptor()
        {
            return Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::InvalidTurnState,
            ));
        }
        Ok(())
    }
}

impl ActiveSession {
    /// Closes a runtime-issued avatar handle owned by this session.
    ///
    /// Cleanup intentionally remains available after session revoke so remote resources are not
    /// stranded. A failed provider close leaves the handle open for retry.
    ///
    /// # Errors
    /// Returns a runtime denial for cross-session/provider handles or a typed provider failure.
    pub fn close_realtime_avatar(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &mut RealtimeAvatarHandle,
    ) -> Result<(), ProviderExecutionError> {
        if handle.session_id != *self.id() || handle.provider != port.descriptor() {
            return Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::InvalidTurnState,
            ));
        }
        if handle.closed {
            return Ok(());
        }
        port.close_session(&handle.provider_session)
            .map_err(ProviderExecutionError::from)?;
        handle.closed = true;
        Ok(())
    }
}
