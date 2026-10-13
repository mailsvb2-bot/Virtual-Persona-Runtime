use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use vpr_domain::{Rt0ReasonCode, SessionId};
use vpr_integration::{
    CancellationProbe, GeneratedAudioBuffer, GeneratedAudioStreamingSink, PcmSampleFormat,
    ProviderDescriptor, ProviderError, ProviderErrorKind, RealtimeAvatarCapability,
    RealtimeAvatarClientCommand, RealtimeAvatarClientControl, RealtimeAvatarClientEvent,
    RealtimeAvatarPcmStream, RealtimeAvatarPort, RealtimeAvatarSession, RealtimeAvatarTransport,
    TtsPort, TtsRequest, WebRtcIceCandidate, WebRtcSessionDescription,
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
    /// Private stop-only capability for server-controlled `LiveKit` avatars.
    #[must_use]
    pub fn backend_stop_handle(
        &self,
        provider: Arc<dyn RealtimeAvatarPort>,
    ) -> Option<RealtimeAvatarStopHandle> {
        if self.closed
            || !matches!(
                self.provider_session.transport,
                RealtimeAvatarTransport::LiveKit { .. }
            )
            || !self
                .client_control
                .as_ref()
                .is_some_and(|control| !control.interrupt && !control.text_input)
        {
            return None;
        }
        Some(RealtimeAvatarStopHandle {
            provider,
            session: self.provider_session.clone(),
        })
    }

    #[must_use]
    pub const fn transport(&self) -> &RealtimeAvatarTransport {
        &self.provider_session.transport
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

/// Stop-only capability independent of the engine mutex. A revoked session
/// can still send a provider STOP, but can never use this handle to speak.
#[derive(Clone)]
pub struct RealtimeAvatarStopHandle {
    provider: Arc<dyn RealtimeAvatarPort>,
    session: RealtimeAvatarSession,
}

struct StopCleanup;
impl CancellationProbe for StopCleanup {
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl RealtimeAvatarStopHandle {
    /// Transmits only a provider interruption; no content can be published.
    ///
    /// # Errors
    /// Reports a typed provider error when STOP is not confirmed.
    pub fn stop(&self) -> Result<(), ProviderExecutionError> {
        self.provider
            .interrupt(&self.session, &StopCleanup)
            .map_err(ProviderExecutionError::from)
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

    fn authorized_realtime_pcm(
        &self,
        tts: &dyn TtsPort,
        text: &str,
    ) -> Result<GeneratedAudioBuffer, ProviderExecutionError> {
        let mut audio = GeneratedAudioBuffer::default();
        self.execute_tts(
            tts,
            &TtsRequest {
                text: text.to_owned(),
                locale_hint: Some("ru".to_owned()),
            },
            &mut audio,
        )?;
        if audio.sample_format() != Some(PcmSampleFormat::S16Le) {
            return Err(ProviderExecutionError::from(
                vpr_integration::ProviderError {
                    kind: vpr_integration::ProviderErrorKind::InvalidResponse,
                    retryable: false,
                },
            ));
        }
        Ok(audio)
    }

    /// Direct avatar speaking through the same canonical TTS permit as
    /// streaming voice, without a browser-controlled D-ID speech route.
    ///
    /// # Errors
    /// Rejects revoked authority and unsupported private audio publishers.
    pub fn speak_realtime_avatar_text_with_tts(
        &self,
        tts: &dyn TtsPort,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<(), ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        let audio = self.authorized_realtime_pcm(tts, text)?;
        let permit = self.avatar_permit()?;
        port.speak_pcm(
            &handle.provider_session,
            audio.pcm(),
            audio.sample_rate_hz().unwrap_or(0),
            audio.channels().unwrap_or(0),
            &permit.cancellation,
        )
        .map_err(ProviderExecutionError::from)
    }

    /// Executes TTS under the canonical turn permit, then sends authorized
    /// PCM directly to a private avatar publisher. Never synthesizes inside
    /// the avatar adapter or exposes audio to browser JavaScript.
    ///
    /// # Errors
    /// Fails closed if either authorization epoch changed, TTS failed, or
    /// the selected avatar does not support private PCM publication.
    pub fn synthesize_and_deliver_realtime_avatar_audio(
        &self,
        tts: &dyn TtsPort,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<OutputDeliveryHandle, RealtimeAvatarOutputError> {
        // Native D-ID Echo documents an explicit pcm16 byte stream. Route
        // streaming TTS chunks through the SAME canonical TTS and avatar permits.
        // Other adapters continue the existing buffered compatibility path.
        if port
            .capabilities()
            .supports(RealtimeAvatarCapability::AudioPcmStreamInput)
        {
            return self.stream_and_deliver_realtime_avatar_pcm(tts, port, handle, text);
        }
        self.validate_avatar_handle(port, handle)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let audio = self
            .authorized_realtime_pcm(tts, text)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        // A fresh permit is mandatory AFTER potentially slow synthesis.
        let permit = self
            .avatar_permit()
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let segment_id = self
            .begin_output_segment()
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        let delivery = OutputDeliveryHandle::new(self.snapshot.turn_id().clone(), segment_id);
        port.speak_pcm(
            &handle.provider_session,
            audio.pcm(),
            audio.sample_rate_hz().unwrap_or(0),
            audio.channels().unwrap_or(0),
            &permit.cancellation,
        )
        .map_err(ProviderExecutionError::from)
        .map_err(RealtimeAvatarOutputError::Provider)?;
        self.acknowledge_output_sent(&delivery)
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        Ok(delivery)
    }

    /// Couples canonical TTS generation to the private avatar's native PCM16
    /// utterance stream. Every provider chunk is validated in a sealed sink,
    /// and every egress checks the SAME revocable avatar permit. A failed TTS
    /// response aborts the provider stream; no incomplete output is marked sent.
    fn stream_and_deliver_realtime_avatar_pcm(
        &self,
        tts: &dyn TtsPort,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<OutputDeliveryHandle, RealtimeAvatarOutputError> {
        self.validate_avatar_handle(port, handle)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let avatar_permit = self
            .avatar_permit()
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let segment_id = self
            .begin_output_segment()
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        let delivery = OutputDeliveryHandle::new(self.snapshot.turn_id().clone(), segment_id);

        let mut published: Option<Box<dyn RealtimeAvatarPcmStream>> = None;
        let mut send = |pcm: &[u8], rate: u32, channels: u16| -> Result<(), ProviderError> {
            if avatar_permit.cancellation.is_cancelled() {
                return Err(ProviderError {
                    kind: ProviderErrorKind::Cancelled,
                    retryable: false,
                });
            }
            if published.is_none() {
                published = Some(port.open_pcm_stream(
                    &handle.provider_session,
                    rate,
                    channels,
                    &avatar_permit.cancellation,
                )?);
            }
            published
                .as_mut()
                .ok_or(ProviderError {
                    kind: ProviderErrorKind::Unavailable,
                    retryable: false,
                })?
                .push_pcm(pcm, &avatar_permit.cancellation)
        };
        let mut sink = GeneratedAudioStreamingSink::new(&mut send);
        let synthesis = self.execute_tts(
            tts,
            &TtsRequest {
                text: text.to_owned(),
                locale_hint: Some("ru".to_owned()),
            },
            &mut sink,
        );
        let received_bytes = sink.received_bytes();
        drop(sink);
        drop(send);

        if let Err(error) = synthesis {
            if let Some(mut stream) = published {
                let _ = stream.abort(); // Explicit provider STOP + transport fencing.
            }
            return Err(RealtimeAvatarOutputError::Provider(error));
        }
        if received_bytes == 0 {
            if let Some(mut stream) = published {
                let _ = stream.abort();
            }
            return Err(RealtimeAvatarOutputError::Provider(
                ProviderExecutionError::from(ProviderError {
                    kind: ProviderErrorKind::InvalidResponse,
                    retryable: false,
                }),
            ));
        }
        let mut stream = published.ok_or_else(|| {
            RealtimeAvatarOutputError::Provider(ProviderExecutionError::from(ProviderError {
                kind: ProviderErrorKind::InvalidResponse,
                retryable: false,
            }))
        })?;
        if let Err(error) = stream.finish(&avatar_permit.cancellation) {
            let _ = stream.abort();
            return Err(RealtimeAvatarOutputError::Provider(
                ProviderExecutionError::from(error),
            ));
        }
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
        playback_id: Option<&str>,
    ) -> Result<RealtimeAvatarClientCommand, ProviderExecutionError> {
        self.validate_avatar_handle(port, handle)?;
        if !handle
            .client_control
            .as_ref()
            .is_some_and(|control| control.interrupt)
        {
            return Err(ProviderExecutionError::Provider(
                vpr_integration::ProviderError {
                    kind: vpr_integration::ProviderErrorKind::Unavailable,
                    retryable: false,
                },
            ));
        }
        let permit = self.avatar_permit()?;
        port.prepare_client_interrupt(&handle.provider_session, playback_id, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Prepares browser-side text delivery for transports whose authenticated realtime connection
    /// lives in the browser. The generated output segment remains unsent until the browser reports
    /// that the transport accepted the command.
    ///
    /// # Errors
    /// Returns a denial for stale authority/handles or a typed provider failure.
    pub fn prepare_realtime_avatar_client_text(
        &self,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<(OutputDeliveryHandle, RealtimeAvatarClientCommand), RealtimeAvatarOutputError>
    {
        self.validate_avatar_handle(port, handle)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let permit = self
            .avatar_permit()
            .map_err(RealtimeAvatarOutputError::Provider)?;
        let segment_id = self
            .begin_output_segment()
            .map_err(RealtimeAvatarOutputError::Runtime)?;
        let delivery = OutputDeliveryHandle::new(self.snapshot.turn_id().clone(), segment_id);
        let command = port
            .prepare_client_text(&handle.provider_session, text, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
            .map_err(RealtimeAvatarOutputError::Provider)?;
        Ok((delivery, command))
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
