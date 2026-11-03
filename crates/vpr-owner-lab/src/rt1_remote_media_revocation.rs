//! Bridges canonical owner-driven visitor revocation to the worker's durable
//! operation-id STOP protocol. No independent authority or media session owner.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

use crate::rt1_visitor_session_manager::VisitorSessionManager;

/// A failed network STOP remains visibly unconfirmed, never equivalent to
/// revocation completing at the remote audio/video sender.
pub struct Rt1RemoteMediaRevocation {
    worker: RemoteOperationClient,
    operation_id: String,
    generation: u64,
    revoked: AtomicBool,
    confirmed: AtomicBool,
    stop_gate: Mutex<()>,
    /// CREATE can return after owner revoke/STOP. Correlate both outcomes to
    /// the exact remote sender, not just the operation/generation envelope.
    created_session_id: Mutex<Option<String>>,
    confirmed_session_id: Mutex<Option<String>>,
}

impl Rt1RemoteMediaRevocation {
    /// # Errors
    /// Rejects invalid operation identity before any owner/visitor registration.
    pub fn new(
        worker: RemoteOperationClient,
        operation_id: String,
        generation: u64,
    ) -> Result<Self, String> {
        if generation == 0
            || operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("invalid remote operation identity".into());
        }
        Ok(Self {
            worker,
            operation_id,
            generation,
            revoked: AtomicBool::new(false),
            confirmed: AtomicBool::new(false),
            stop_gate: Mutex::new(()),
            created_session_id: Mutex::new(None),
            confirmed_session_id: Mutex::new(None),
        })
    }

    /// Bind a late CREATE reply to the operation before publishing its SDP.
    /// A STOP receipt for a different sender cannot authorize that CREATE.
    /// The STOP gate serializes this binding against remote cancellation.
    ///
    /// # Errors
    /// Refuses conflicting identities, including STOP-before-CREATE races.
    pub fn record_created_session(&self, session_id: &str) -> Result<(), String> {
        if session_id.trim().is_empty() {
            return Err("RT1_REMOTE_SESSION_ID_INVALID".into());
        }
        let _serial = self.stop_gate.lock();
        let mut created = self.created_session_id.lock();
        if created.as_deref().is_some_and(|previous| previous != session_id)
            || self
                .confirmed_session_id
                .lock()
                .as_deref()
                .is_some_and(|confirmed| confirmed != session_id)
        {
            self.revoked.store(true, Ordering::Release);
            self.confirmed.store(false, Ordering::Release);
            return Err("RT1_REMOTE_SESSION_ID_CONFLICT".into());
        }
        *created = Some(session_id.to_owned());
        Ok(())
    }

    /// Invoked by the existing canonical visitor stop callback, after the
    /// owner publication transaction is released. No engine mutex is held.
    /// An uncertain outcome stays unconfirmed and can be reconciled later.
    ///
    /// # Errors
    /// No remote sender STOP receipt, stale generation or transport timeout.
    pub fn revoke_confirmed(&self) -> Result<(), String> {
        self.revoked.store(true, Ordering::Release);
        let _serial = self.stop_gate.lock();
        if self.confirmed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cancel_locked()
    }

    // Called with stop_gate held. Both owner revoke and failed SDP answer
    // require the same independent audio/video STOP receipt.
    fn cancel_locked(&self) -> Result<(), String> {
        if self.confirmed.load(Ordering::Acquire) {
            return Ok(());
        }
        let stopped_session = self
            .worker
            .cancel_confirmed(&self.operation_id, self.generation)
            .map_err(|_| "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())?;
        if self
            .created_session_id
            .lock()
            .as_deref()
            .is_some_and(|created| created != stopped_session)
        {
            // The worker acknowledged STOP for a different sender. Preserve
            // quarantine; never advertise confirmed audio/video termination.
            return Err("RT1_REMOTE_MEDIA_STOP_IDENTITY_MISMATCH".into());
        }
        *self.confirmed_session_id.lock() = Some(stopped_session);
        self.confirmed.store(true, Ordering::Release);
        Ok(())
    }

    /// Submit SDP only while the operation is not revoked.
    /// # Errors
    /// Denies revoked operations and remote signaling failures.
    pub fn submit_answer(&self, sdp: &str) -> Result<(), String> {
        let _serial = self.stop_gate.lock();
        if self.revoked.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        if self.worker.answer(&self.operation_id, sdp).is_err() {
            // A rejected or lost SDP receipt does not prove the provider
            // never accepted it. Quarantine and compensate under the same
            // signaling/STOP gate, preventing any second answer.
            self.revoked.store(true, Ordering::Release);
            self.cancel_locked()?;
            return Err("RT1_REMOTE_ANSWER_UNCONFIRMED".to_owned());
        }
        // Owner revoke marks this flag before acquiring stop_gate. If it
        // arrived during the in-flight HTTP request, a late provider ACK must
        // not be exposed to the visitor as successful authorization.
        if self.revoked.load(Ordering::Acquire) {
            // The owner may have revoked authority while the provider was
            // accepting SDP. Complete the compensating STOP *before* the
            // signaling caller returns: a delayed owner callback must not be
            // the only path to shutting down an already accepted answer.
            // Keep the operation quarantined when either sender is uncertain.
            self.cancel_locked()?;
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(())
    }

    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_stop_confirmed(&self) -> bool {
        self.confirmed.load(Ordering::Acquire)
    }
}

impl VisitorSessionManager {
    /// Register exactly one remote operation STOP with the existing visitor
    /// authority. Caller must retain the returned bridge to inspect or retry
    /// an uncertain remote shutdown. No new permission model is introduced.
    ///
    /// # Errors
    /// Denies visitor admission unless the current Persona is published.
    pub fn open_with_remote_operation(
        &self,
        visitor_token: String,
        remote: Arc<Rt1RemoteMediaRevocation>,
    ) -> Result<String, String> {
        self.open(visitor_token, move || {
            // A failed STOP is not a success: remote.confirmed remains false.
            // Worker operation records remain durable and must be reconciled.
            let _ = remote.revoke_confirmed();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[test]
    fn remote_stop_pending_never_confirms_and_can_be_reconciled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let read = socket.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|segment| segment == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(request.starts_with(b"DELETE /v1/avatar/operations/pending-stop "));
                let body = if attempt == 0 {
                    r#"{"operation_id":"pending-stop","generation":3,"session_id":"remote-session","audio_stopped":false,"video_stopped":false}"#
                } else {
                    r#"{"operation_id":"pending-stop","generation":3,"session_id":"remote-session","audio_stopped":true,"video_stopped":true}"#
                };
                let status = if attempt == 0 {
                    "202 Accepted"
                } else {
                    "200 OK"
                };
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "pending-stop".into(), 3).unwrap();
        assert!(!remote.is_revoked());
        assert!(!remote.is_stop_confirmed());
        assert_eq!(
            remote.revoke_confirmed().unwrap_err(),
            "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"
        );
        assert!(remote.is_revoked());
        assert!(!remote.is_stop_confirmed());
        remote.revoke_confirmed().unwrap();
        assert!(remote.is_stop_confirmed());
        worker.join().unwrap();
    }

    #[test]
    fn owner_revoke_during_remote_answer_never_returns_stale_success() {
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (answer_received_tx, answer_received_rx) = mpsc::channel();
        let (release_answer_tx, release_answer_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = socket.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with(b"POST /v1/avatar/operations/raced-answer/answer "));
            answer_received_tx.send(()).unwrap();
            release_answer_rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            let body = r#"{"operation_id":"raced-answer","accepted":true}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();

            let (mut stop_socket, _) = listener.accept().unwrap();
            stop_socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let read = stop_socket.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with(b"DELETE /v1/avatar/operations/raced-answer "));
            let body = r#"{"operation_id":"raced-answer","generation":1,"session_id":"remote-session","audio_stopped":true,"video_stopped":true}"#;
            write!(
                stop_socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        });
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote =
            Arc::new(Rt1RemoteMediaRevocation::new(client, "raced-answer".into(), 1).unwrap());
        let answer_remote = Arc::clone(&remote);
        let answer = std::thread::spawn(move || answer_remote.submit_answer("v=0\r\n"));
        answer_received_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let stop_remote = Arc::clone(&remote);
        let stop = std::thread::spawn(move || stop_remote.revoke_confirmed());
        // revoke_confirmed() sets the revoked bit before waiting on the
        // ongoing answer's stop_gate. Observe it without assuming scheduling.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !remote.is_revoked() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(remote.is_revoked());
        release_answer_tx.send(()).unwrap();
        assert_eq!(answer.join().unwrap().unwrap_err(), "RT1_VISITOR_REVOKED");
        stop.join().unwrap().unwrap();
        assert!(remote.is_stop_confirmed());
        worker.join().unwrap();
    }

    #[test]
    fn raced_accepted_answer_compensates_stop_before_return_and_preserves_uncertainty() {
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            for attempt in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let n = socket.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                let (status, body) = if attempt == 0 {
                    assert!(
                        request.starts_with(b"POST /v1/avatar/operations/raced-pending/answer ")
                    );
                    accepted_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    (
                        "200 OK",
                        r#"{"operation_id":"raced-pending","accepted":true}"#,
                    )
                } else {
                    assert!(request.starts_with(b"DELETE /v1/avatar/operations/raced-pending "));
                    if attempt == 1 {
                        (
                            "202 Accepted",
                            r#"{"operation_id":"raced-pending","stop_confirmed":false}"#,
                        )
                    } else {
                        (
                            "200 OK",
                            r#"{"operation_id":"raced-pending","generation":1,"session_id":"sender","audio_stopped":true,"video_stopped":true}"#,
                        )
                    }
                };
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote =
            Arc::new(Rt1RemoteMediaRevocation::new(client, "raced-pending".into(), 1).unwrap());
        let submitting = Arc::clone(&remote);
        let answer = std::thread::spawn(move || submitting.submit_answer("v=0\r\n"));
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        // Model the owner's revocation flag becoming visible while the SDP
        // request holds the signaling/STOP gate; do not start another runtime.
        remote.revoked.store(true, Ordering::Release);
        release_tx.send(()).unwrap();
        assert_eq!(
            answer.join().unwrap().unwrap_err(),
            "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"
        );
        assert!(!remote.is_stop_confirmed());
        assert!(remote.is_revoked());
        remote.revoke_confirmed().unwrap();
        assert!(remote.is_stop_confirmed());
        worker.join().unwrap();
    }

    #[test]
    fn negative_answer_receipt_denies_signaling_and_still_allows_verified_revoke() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for (expected_path, body) in [
                (
                    "POST /v1/avatar/operations/negative-answer/answer ",
                    r#"{"operation_id":"negative-answer","accepted":false}"#,
                ),
                (
                    "DELETE /v1/avatar/operations/negative-answer ",
                    r#"{"operation_id":"negative-answer","generation":4,"session_id":"sender-4","audio_stopped":true,"video_stopped":true}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let read = socket.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(request.starts_with(expected_path.as_bytes()));
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "negative-answer".into(), 4).unwrap();
        assert_eq!(
            remote.submit_answer("v=0\r\n").unwrap_err(),
            "RT1_REMOTE_ANSWER_UNCONFIRMED"
        );
        // An SDP failure must trigger compensating STOP without a browser
        // close or a second owner revoke.
        assert!(remote.is_revoked());
        assert!(remote.is_stop_confirmed());
        assert_eq!(
            remote.submit_answer("v=0\r\n").unwrap_err(),
            "RT1_VISITOR_REVOKED"
        );
        worker.join().unwrap();
    }

    #[test]
    fn failed_answer_with_pending_stop_stays_quarantined_until_retry_receipt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for (method, status, body) in [
                (
                    "POST",
                    "200 OK",
                    r#"{"operation_id":"lost-answer","accepted":false}"#,
                ),
                (
                    "DELETE",
                    "202 Accepted",
                    r#"{"operation_id":"lost-answer","generation":8,"session_id":"sender-8","audio_stopped":false,"video_stopped":false}"#,
                ),
                (
                    "DELETE",
                    "200 OK",
                    r#"{"operation_id":"lost-answer","generation":8,"session_id":"sender-8","audio_stopped":true,"video_stopped":true}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let read = socket.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                let expected_path = if method == "POST" {
                    "POST /v1/avatar/operations/lost-answer/answer "
                } else {
                    "DELETE /v1/avatar/operations/lost-answer "
                };
                assert!(request.starts_with(expected_path.as_bytes()));
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "lost-answer".into(), 8).unwrap();
        assert_eq!(
            remote.submit_answer("v=0\r\n").unwrap_err(),
            "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"
        );
        assert!(remote.is_revoked());
        assert!(!remote.is_stop_confirmed());
        assert_eq!(
            remote.submit_answer("v=0\r\n").unwrap_err(),
            "RT1_VISITOR_REVOKED"
        );
        remote.revoke_confirmed().unwrap();
        assert!(remote.is_stop_confirmed());
        worker.join().unwrap();
    }

    #[test]
    fn remote_operation_identity_must_be_valid_before_admission() {
        let client = RemoteOperationClient::new(
            "http://127.0.0.1:42119",
            "worker-secret",
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(Rt1RemoteMediaRevocation::new(client.clone(), "bad/id".into(), 1).is_err());
        assert!(Rt1RemoteMediaRevocation::new(client, "safe-operation".into(), 0).is_err());
    }

    fn single_stop_receipt(
        operation_id: &'static str,
        generation: u64,
        sender_id: &'static str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let n = socket.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with(
                format!("DELETE /v1/avatar/operations/{operation_id} ").as_bytes()
            ));
            let body = serde_json::json!({
                "operation_id": operation_id,
                "generation": generation,
                "session_id": sender_id,
                "audio_stopped": true,
                "video_stopped": true,
            })
            .to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (endpoint, worker)
    }

    #[test]
    fn wrong_sender_stop_receipt_cannot_confirm_remote_audio_or_video() {
        let (endpoint, worker) = single_stop_receipt("mismatched-stop", 2, "other-sender");
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "mismatched-stop".into(), 2).unwrap();
        remote.record_created_session("created-sender").unwrap();
        assert_eq!(
            remote.revoke_confirmed().unwrap_err(),
            "RT1_REMOTE_MEDIA_STOP_IDENTITY_MISMATCH"
        );
        assert!(remote.is_revoked());
        assert!(!remote.is_stop_confirmed());
        worker.join().unwrap();
    }

    #[test]
    fn late_create_after_early_stop_must_match_confirmed_sender() {
        let (endpoint, worker) = single_stop_receipt("late-create", 4, "stopped-sender");
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "late-create".into(), 4).unwrap();
        remote.revoke_confirmed().unwrap();
        assert!(remote.is_stop_confirmed());
        assert_eq!(
            remote.record_created_session("late-different-sender").unwrap_err(),
            "RT1_REMOTE_SESSION_ID_CONFLICT"
        );
        assert!(!remote.is_stop_confirmed());
        assert!(remote.is_revoked());
        worker.join().unwrap();
    }

    #[test]
    fn late_create_matching_early_stop_preserves_confirmed_shutdown() {
        let (endpoint, worker) = single_stop_receipt("matching-late", 5, "same-sender");
        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
        let remote = Rt1RemoteMediaRevocation::new(client, "matching-late".into(), 5).unwrap();
        remote.revoke_confirmed().unwrap();
        remote.record_created_session("same-sender").unwrap();
        assert!(remote.is_stop_confirmed());
        worker.join().unwrap();
    }
}
