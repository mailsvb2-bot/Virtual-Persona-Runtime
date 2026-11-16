//! The existing RT1 durable worker as a canonical `RealtimeAvatarPort`.
//! It owns no publication authority: the caller pre-registers this operation
//! under the existing RT1 owner/visitor revocation and STOP coordinator.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
type CreatedSessionObserver = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
type RemoteStopObserver = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub struct OperationWorkerAvatar {
    client: RemoteOperationClient,
    operation_id: String,
    generation: u64,
    create_attempted: AtomicBool,
    created_session: Mutex<Option<String>>,
    created_session_observer: Option<CreatedSessionObserver>,
    remote_stop_observer: Option<RemoteStopObserver>,
}

impl OperationWorkerAvatar {
    /// # Errors
    /// Rejects malformed identity before any CREATE or external provider I/O.
    pub fn new(
        client: RemoteOperationClient,
        operation_id: String,
        generation: u64,
    ) -> Result<Self, ProviderError> {
        Self::build(client, operation_id, generation, None, None)
    }

    /// Bind the adapter to the visitor's already registered canonical remote
    /// lifecycle owner. The observer records every late CREATE id before SDP
    /// offer publication; the STOP callback delegates to that same owner.
    ///
    /// # Errors
    /// Rejects malformed operation identity before any provider I/O.
    pub fn with_remote_owner(
        client: RemoteOperationClient,
        operation_id: String,
        generation: u64,
        created_session: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
        remote_stop: impl Fn() -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<Self, ProviderError> {
        Self::build(
            client,
            operation_id,
            generation,
            Some(Arc::new(created_session)),
            Some(Arc::new(remote_stop)),
        )
    }

    fn build(
        client: RemoteOperationClient,
        operation_id: String,
        generation: u64,
        created_session_observer: Option<CreatedSessionObserver>,
        remote_stop_observer: Option<RemoteStopObserver>,
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
            created_session_observer,
            remote_stop_observer,
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
        if cancellation.is_cancelled() {
            return Err(denied());
        }
        if cancellation.is_cancelled() || self.create_attempted.swap(true, Ordering::AcqRel) {
            return Err(denied());
        }
        // CREATE is never retried after an ambiguous remote outcome. If
        // cancellation races the durable PUT, return its late id to the same
        // registered owner before any SDP offer can be fetched or published.
        let id =
            self.client
                .create_cancellable(&self.operation_id, self.generation, cancellation)?;
        if let Some(observer) = &self.created_session_observer {
            observer(&id).map_err(|_| denied())?;
        }
        if cancellation.is_cancelled() {
            return Err(denied());
        }
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
        let mut fragments = audio.chunks(16_000).peekable();
        while let Some(chunk) = fragments.next() {
            if cancellation.is_cancelled() {
                return Err(denied());
            }
            if fragments.peek().is_none() {
                // This is the actual end of this canonical TTS utterance;
                // flush the renderer's remaining 20-380 ms PCM as part of
                // the SAME authenticated worker/media operation.
                self.client.push_pcm_final(&self.operation_id, chunk)?;
            } else {
                self.client.push_pcm(&self.operation_id, chunk)?;
            }
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
        if let Some(stop) = &self.remote_stop_observer {
            // RT1 uses one canonical Rt1RemoteMediaRevocation owner. Do not
            // race an independent adapter DELETE against the owner reconciler.
            stop().map_err(|_| denied())
        } else {
            let confirmed_id = self
                .client
                .cancel_confirmed(&self.operation_id, self.generation)?;
            if confirmed_id != session.provider_session_id {
                return Err(denied());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OperationWorkerAvatar;
    use crate::operation_client::RemoteOperationClient;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use vpr_integration::{CancellationProbe, RealtimeAvatarPort};

    struct ActiveTurn;
    impl CancellationProbe for ActiveTurn {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    fn read_request_line(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set bounded test socket timeout");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let count = stream.read(&mut chunk).expect("read test HTTP request");
            assert_ne!(count, 0, "HTTP request ended before headers");
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            assert!(bytes.len() <= 16_384, "test request headers exceeded bound");
        }
        String::from_utf8(bytes)
            .expect("HTTP request is ASCII")
            .lines()
            .next()
            .expect("HTTP request line")
            .to_owned()
    }

    fn write_response(stream: &mut TcpStream, status: &str, body: &str) {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("write test HTTP response");
    }

    #[test]
    fn unavailable_persona_renderer_blocks_worker_create_for_existing_invites() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback test worker");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("test worker address");
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (mut first, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "worker readiness request not received"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept readiness request: {error}"),
                }
            };
            let mut requests = vec![read_request_line(&mut first)];
            write_response(
                &mut first,
                "503 Service Unavailable",
                r#"{"ready":false,"persona_video_ready":false}"#,
            );
            drop(first);

            // Observe briefly for a forbidden follow-up PUT. A correct client
            // stops after the renderer-readiness denial and never allocates.
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        requests.push(read_request_line(&mut stream));
                        write_response(&mut stream, "409 Conflict", r#"{"error":"denied"}"#);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept follow-up request: {error}"),
                }
            }
            requests
        });

        let client = RemoteOperationClient::new(
            &format!("http://{address}"),
            "test-secret",
            Duration::from_secs(2),
        )
        .expect("build loopback worker client");
        let avatar = OperationWorkerAvatar::new(client, "existing-invite".to_owned(), 1)
            .expect("build canonical worker avatar");
        assert!(
            avatar.create_session(&ActiveTurn).is_err(),
            "renderer-unready worker must deny visitor CREATE",
        );
        assert_eq!(
            server.join().expect("worker test thread"),
            vec!["GET /v1/avatar/readiness HTTP/1.1"],
            "renderer denial must prevent the follow-up media CREATE",
        );
    }

    #[test]
    fn close_session_delegates_to_the_registered_remote_stop_owner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use vpr_integration::{
            RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcSessionDescription,
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let stop_calls = Arc::clone(&calls);
        let client = RemoteOperationClient::new(
            "http://127.0.0.1:1",
            "worker-secret",
            Duration::from_millis(100),
        )
        .unwrap();
        let avatar = OperationWorkerAvatar::with_remote_owner(
            client,
            "operation-1".to_owned(),
            1,
            |session_id| {
                assert_eq!(session_id, "media-1");
                Ok(())
            },
            move || {
                stop_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        *avatar.created_session.lock().unwrap() = Some("media-1".to_owned());
        let session = RealtimeAvatarSession {
            provider_resource_id: "operation-1".to_owned(),
            provider_session_id: "media-1".to_owned(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: "offer".to_owned(),
                    sdp: "offer-sdp".to_owned(),
                },
                ice_servers: Vec::new(),
            },
        };

        avatar.close_session(&session).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn canonical_tts_pcm_resamples_to_bounded_worker_frames() {
        let samples: Vec<u8> = (0..24_000)
            .flat_map(|index| {
                i16::try_from(index)
                    .expect("test sample fits i16")
                    .to_le_bytes()
            })
            .collect();
        let result = OperationWorkerAvatar::normalize_pcm(&samples, 24_000, 1).unwrap();
        assert_eq!(result.len(), 32_000);
        assert!(result.iter().any(|byte| *byte != 0));
        assert!(OperationWorkerAvatar::normalize_pcm(&samples, 24_000, 2).is_err());
    }
}
