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
        })
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
        self.worker
            .cancel_confirmed(&self.operation_id, self.generation)
            .map_err(|_| "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())?;
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
        self.worker
            .answer(&self.operation_id, sdp)
            .map_err(|_| "RT1_REMOTE_ANSWER_UNCONFIRMED".to_owned())?;
        // Owner revoke marks this flag before acquiring stop_gate. If it
        // arrived during the in-flight HTTP request, a late provider ACK must
        // not be exposed to the visitor as successful authorization.
        if self.revoked.load(Ordering::Acquire) {
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
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
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
            release_answer_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let body = r#"{"operation_id":"raced-answer","accepted":true}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();

            let (mut stop_socket, _) = listener.accept().unwrap();
            stop_socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
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
        let remote = Arc::new(
            Rt1RemoteMediaRevocation::new(client, "raced-answer".into(), 1).unwrap(),
        );
        let answer_remote = Arc::clone(&remote);
        let answer = std::thread::spawn(move || answer_remote.submit_answer("v=0\r\n"));
        answer_received_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let stop_remote = Arc::clone(&remote);
        let stop = std::thread::spawn(move || stop_remote.revoke_confirmed());
        // revoke_confirmed() sets the revoked bit before waiting on the
        // ongoing answer's stop_gate. Observe it without assuming scheduling.
        for _ in 0..1000 {
            if remote.is_revoked() {
                break;
            }
            std::thread::yield_now();
        }
        assert!(remote.is_revoked());
        release_answer_tx.send(()).unwrap();
        assert_eq!(answer.join().unwrap().unwrap_err(), "RT1_VISITOR_REVOKED");
        stop.join().unwrap().unwrap();
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
}
