//! The existing RT1 durable worker as a canonical `RealtimeAvatarPort`.
//! It owns no publication authority: the caller pre-registers this operation
//! under the existing RT1 owner/visitor revocation and STOP coordinator.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription,
};

use crate::operation_client::RemoteOperationClient;

fn denied() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: false,
    }
}

/// A single worker-backed avatar session driven by the existing canonical
/// `OwnerLabEngine`. CREATE is non-retryable; only the worker ledger owns STOP.
pub struct OperationWorkerAvatar {
    client: RemoteOperationClient,
    operation_id: String,
    generation: u64,
    create_attempted: AtomicBool,
    created_session: Mutex<Option<String>>,
}

impl OperationWorkerAvatar {
    /// # Errors
    /// Rejects malformed identity before any CREATE or external provider I/O.
    pub fn new(
        client: RemoteOperationClient,
        operation_id: String,
        generation: u64,
    ) -> Result<Self, ProviderError> {
        if generation == 0
            || operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_')
        {
            return Err(denied());
        }
        Ok(Self {
            client,
            operation_id,
            generation,
            create_attempted: AtomicBool::new(false),
            created_session: Mutex::new(None),
        })
    }

    fn validate_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        if session.provider_resource_id != self.operation_id
            || !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. })
            || self
                .created_session
                .lock()
                .map_err(|_| denied())?
                .as_deref()
                != Some(session.provider_session_id.as_str())
        {
            return Err(denied());
        }
        Ok(())
    }

    // The Worker HTTP boundary accepts exact 20 ms PCM16, 16 kHz, mono.
    // Resample already-authorized canonical TTS audio, never synthesize here.
    fn normalize_pcm(pcm: &[u8], rate: u32, channels: u16) -> Result<Vec<u8>, ProviderError> {
        if channels != 1
            || !(8_000..=48_000).contains(&rate)
            || pcm.len() < 2
            || pcm.len() % 2 != 0
            || pcm.len() / 2 > usize::try_from(rate).map_err(|_| denied())? * 30
        {
            return Err(denied());
        }
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect();
        let output_len = usize::try_from(
            u64::try_from(samples.len()).map_err(|_| denied())? * 16_000 / u64::from(rate),
        )
        .map_err(|_| denied())?;
        if output_len == 0 || output_len > 480_000 {
            return Err(denied());
        }
        let mut output = Vec::with_capacity(output_len.div_ceil(320) * 640);
        for index in 0..output_len {
            let position = u64::try_from(index).map_err(|_| denied())? * u64::from(rate);
            let left = (position / 16_000) as usize;
            let fraction = i64::try_from(position % 16_000).map_err(|_| denied())?;
            let a = i64::from(samples[left]);
            let next = samples.get(left + 1).copied().unwrap_or(samples[left]);
            let b = i64::from(next);
            let interpolated = (a * (16_000 - fraction) + b * fraction) / 16_000;
            output.extend_from_slice(
                &i16::try_from(interpolated)
                    .map_err(|_| denied())?
                    .to_le_bytes(),
            );
        }
        while output.len() % 640 != 0 {
            output.push(0);
        }
        Ok(output)
    }
}

impl RealtimeAvatarPort for OperationWorkerAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "rt1-operation-worker".to_owned(),
            model: "durable-aiortc-pcm".to_owned(),
            representation: None,
        }
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([RealtimeAvatarCapability::AudioPcmInput])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        if cancellation.is_cancelled() || self.create_attempted.swap(true, Ordering::AcqRel) {
            return Err(denied());
        }
        // CREATE is never retried after an ambiguous remote outcome.
        let id = self.client.create(&self.operation_id, self.generation)?;
        let offer = self.client.offer(&self.operation_id, &id)?;
        if cancellation.is_cancelled() {
            return Err(denied()); // registered owner STOP/reconciler owns cleanup
        }
        let mut started = self.created_session.lock().map_err(|_| denied())?;
        *started = Some(id.clone());
        Ok(RealtimeAvatarSession {
            provider_resource_id: self.operation_id.clone(),
            provider_session_id: id,
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: offer.kind,
                    sdp: offer.sdp,
                },
                ice_servers: offer
                    .ice_servers
                    .into_iter()
                    .map(|server| WebRtcIceServer {
                        urls: server.urls,
                        username: server.username,
                        credential: server.credential,
                    })
                    .collect(),
            },
        })
    }

    fn submit_answer(
        &self,
        session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.validate_session(session)?;
        if cancellation.is_cancelled() || answer.kind != "answer" {
            return Err(denied());
        }
        self.client.answer(&self.operation_id, &answer.sdp)?;
        if cancellation.is_cancelled() {
            return Err(denied()); // owner revocation drives confirmed STOP
        }
        Ok(())
    }

    fn submit_ice_candidate(
        &self,
        session: &RealtimeAvatarSession,
        candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.validate_session(session)?;
        // aiortc gathers ICE before publishing its offer. Never falsely ACK
        // nontrickle candidates that the operation worker cannot apply.
        if cancellation.is_cancelled() || candidate.candidate.is_some() {
            return Err(denied());
        }
        Ok(())
    }

    fn speak_text(
        &self,
        _session: &RealtimeAvatarSession,
        _text: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        // TTS must remain the canonical server-owned STT→LLM→TTS path.
        Err(denied())
    }

    fn speak_pcm(
        &self,
        session: &RealtimeAvatarSession,
        pcm: &[u8],
        sample_rate_hz: u32,
        channels: u16,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.validate_session(session)?;
        let audio = Self::normalize_pcm(pcm, sample_rate_hz, channels)?;
        // Match worker 2-second queue: send half-second chunks with bounded
        // pacing and recheck the SAME revocable canonical turn on every write.
        for chunk in audio.chunks(16_000) {
            if cancellation.is_cancelled() {
                return Err(denied());
            }
            self.client.push_pcm(&self.operation_id, chunk)?;
            if cancellation.is_cancelled() {
                return Err(denied());
            }
            thread::sleep(Duration::from_millis(480));
        }
        Ok(())
    }

    fn speak_audio_url(
        &self,
        _session: &RealtimeAvatarSession,
        _audio_url: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Err(denied())
    }

    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        self.validate_session(session)?;
        let confirmed_id = self
            .client
            .cancel_confirmed(&self.operation_id, self.generation)?;
        if confirmed_id != session.provider_session_id {
            return Err(denied());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::OperationWorkerAvatar;

    #[test]
    fn canonical_tts_pcm_resamples_to_bounded_worker_frames() {
        let samples: Vec<u8> = (0..24_000)
            .flat_map(|index| i16::try_from(index).expect("test sample fits i16").to_le_bytes())
            .collect();
        let result = OperationWorkerAvatar::normalize_pcm(&samples, 24_000, 1).unwrap();
        assert_eq!(result.len(), 32_000);
        assert!(result.iter().any(|byte| *byte != 0));
        assert!(OperationWorkerAvatar::normalize_pcm(&samples, 24_000, 2).is_err());
    }
}
