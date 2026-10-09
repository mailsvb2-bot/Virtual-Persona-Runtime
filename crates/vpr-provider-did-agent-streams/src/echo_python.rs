//! Explicitly enabled, backend-only `LiveKit` Echo sender.
//! No room publisher token, TTS key or generated audio reaches browser JSON.
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use vpr_integration::{CancellationProbe, ProviderError};

use super::{DidEchoBackend, cancelled, invalid_response, unavailable};

const OPEN_TIMEOUT: Duration = Duration::from_secs(55);
const SPEECH_TIMEOUT: Duration = Duration::from_secs(80);
// A missing vendor STOP receipt must never delay local safety fencing for five seconds.
// Successful dispatch is not proof of remote audible playback completion.
const INTERRUPT_TIMEOUT: Duration = Duration::from_millis(350);
const MAX_PHRASE_BYTES: usize = 4096;
const MAX_RECEIPT_BYTES: usize = 512;

#[derive(Clone)]
pub struct EchoPythonConfig {
    python_executable: String,
    tts_endpoint: String,
    tts_api_key: String,
    tts_model: String,
    tts_voice: String,
}

impl EchoPythonConfig {
    /// Creates an explicitly configured and credentialed private Echo backend.
    ///
    /// # Errors
    /// Refuses absent interpreter, invalid HTTPS TTS endpoint or missing credentials.
    pub fn new(
        python_executable: impl Into<String>,
        tts_endpoint: impl Into<String>,
        tts_api_key: impl Into<String>,
        tts_model: impl Into<String>,
        tts_voice: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let config = Self {
            python_executable: python_executable.into(),
            tts_endpoint: tts_endpoint.into(),
            tts_api_key: tts_api_key.into(),
            tts_model: tts_model.into(),
            tts_voice: tts_voice.into(),
        };
        let url = reqwest::Url::parse(&config.tts_endpoint).map_err(|_| invalid_response())?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || [
                &config.python_executable,
                &config.tts_api_key,
                &config.tts_model,
                &config.tts_voice,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(invalid_response());
        }
        Ok(config)
    }
}

/// Private subprocess transport. One child/`LiveKit` sender per canonical session.
/// The child carries a cancellation epoch and serializes utterance byte streams.
pub struct EchoPythonBackend {
    config: EchoPythonConfig,
    sessions: Mutex<HashMap<String, Arc<SessionWorker>>>,
}

#[derive(Deserialize)]
struct Receipt {
    id: u64,
    ok: bool,
}

struct SessionWorker {
    child: Mutex<Child>,
    command_tx: mpsc::SyncSender<Vec<u8>>,
    // Protect only producer ordering and STOP fencing, never blocking pipe I/O.
    enqueue_gate: Mutex<()>,
    stop_gate: Mutex<()>,
    stop_receipt: Mutex<Option<bool>>,
    pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<bool>>>>,
    next_id: AtomicU64,
    stopped: AtomicBool,
    disconnected: Arc<AtomicBool>,
}

impl SessionWorker {
    fn spawn(
        config: &EchoPythonConfig,
        session_url: &str,
        token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Arc<Self>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let mut child = Command::new(&config.python_executable)
            .arg("-u")
            .arg("-c")
            .arg(include_str!("echo_python_worker.py"))
            .env("VPR_DID_ECHO_TTS_ENDPOINT", &config.tts_endpoint)
            .env("VPR_DID_ECHO_TTS_API_KEY", &config.tts_api_key)
            .env("VPR_DID_ECHO_TTS_MODEL", &config.tts_model)
            .env("VPR_DID_ECHO_TTS_VOICE", &config.tts_voice)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable())?;
        let stdin = child.stdin.take().ok_or_else(unavailable)?;
        let stdout = child.stdout.take().ok_or_else(unavailable)?;
        let pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<bool>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let disconnected = Arc::new(AtomicBool::new(false));
        // Only the dedicated writer owns the blocking OS pipe. STOP never waits
        // for a stalled speak to acquire a stdin mutex: all producers enqueue
        // bounded commands without blocking, and can kill the child independently.
        let (command_tx, commands) = mpsc::sync_channel::<Vec<u8>>(16);
        thread::spawn(move || {
            let mut stdin: ChildStdin = stdin;
            while let Ok(message) = commands.recv() {
                if stdin.write_all(&message).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let worker = Arc::new(Self {
            child: Mutex::new(child),
            command_tx,
            enqueue_gate: Mutex::new(()),
            stop_gate: Mutex::new(()),
            stop_receipt: Mutex::new(None),
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            disconnected: Arc::clone(&disconnected),
        });
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            'receipts: loop {
                let mut line = Vec::with_capacity(64);
                loop {
                    let mut byte = [0_u8; 1];
                    if stdout.read(&mut byte).ok() != Some(1) {
                        break 'receipts;
                    }
                    if byte[0] == b'\n' {
                        break;
                    }
                    if line.len() >= MAX_RECEIPT_BYTES {
                        break 'receipts;
                    }
                    line.push(byte[0]);
                }
                if let Ok(receipt) = serde_json::from_slice::<Receipt>(&line) {
                    if let Ok(mut requests) = pending.lock() {
                        if let Some(reply) = requests.remove(&receipt.id) {
                            let _ = reply.send(receipt.ok);
                        }
                    }
                }
            }
            disconnected.store(true, Ordering::Release);
            if let Ok(mut requests) = pending.lock() {
                for (_, reply) in requests.drain() {
                    let _ = reply.send(false);
                }
            }
        });
        let result = worker.request(
            json!({"command": "open", "session_url": session_url, "echo_token": token}),
            OPEN_TIMEOUT,
            Some(cancellation),
            true,
        );
        if let Err(error) = result {
            let _ = worker.terminate();
            return Err(error);
        }
        Ok(worker)
    }

    fn request(
        &self,
        mut payload: Value,
        timeout: Duration,
        cancellation: Option<&dyn CancellationProbe>,
        allow_stopped: bool,
    ) -> Result<(), ProviderError> {
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let _enqueue = self.enqueue_gate.lock().map_err(|_| unavailable())?;
            if self.disconnected.load(Ordering::Acquire)
                || (!allow_stopped && self.stopped.load(Ordering::Acquire))
                || cancellation.is_some_and(CancellationProbe::is_cancelled)
            {
                return Err(cancelled());
            }
            payload["id"] = json!(id);
            let mut encoded = serde_json::to_vec(&payload).map_err(|_| invalid_response())?;
            encoded.push(b'\n');
            self.pending
                .lock()
                .map_err(|_| unavailable())?
                .insert(id, sender);
            if self.command_tx.try_send(encoded).is_err() {
                let _ = self.pending.lock().map(|mut pending| pending.remove(&id));
                return Err(unavailable());
            }
        }
        let deadline = Instant::now() + timeout;
        loop {
            if cancellation.is_some_and(CancellationProbe::is_cancelled) {
                let _ = self.pending.lock().map(|mut pending| pending.remove(&id));
                return Err(cancelled());
            }
            if Instant::now() >= deadline {
                let _ = self.pending.lock().map(|mut pending| pending.remove(&id));
                return Err(unavailable());
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => return Err(unavailable()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn mark_stopped(&self) -> Result<(), ProviderError> {
        let _enqueue = self.enqueue_gate.lock().map_err(|_| unavailable())?;
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }

    fn terminate(&self) -> Result<(), ProviderError> {
        self.mark_stopped()?;
        let mut child = self.child.lock().map_err(|_| unavailable())?;
        if child.try_wait().map_err(|_| unavailable())?.is_none() {
            child.kill().map_err(|_| unavailable())?;
            child.wait().map_err(|_| unavailable())?;
        }
        Ok(())
    }
}

impl EchoPythonBackend {
    #[must_use]
    pub fn new(config: EchoPythonConfig) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn worker(&self, id: &str) -> Result<Arc<SessionWorker>, ProviderError> {
        self.sessions
            .lock()
            .map_err(|_| unavailable())?
            .get(id)
            .cloned()
            .ok_or_else(unavailable)
    }
}

impl DidEchoBackend for EchoPythonBackend {
    fn open(
        &self,
        session_id: &str,
        session_url: &str,
        echo_token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        if self
            .sessions
            .lock()
            .map_err(|_| unavailable())?
            .contains_key(session_id)
        {
            return Err(invalid_response());
        }
        let worker = SessionWorker::spawn(&self.config, session_url, echo_token, cancellation)?;
        if cancellation.is_cancelled() {
            worker.terminate()?;
            return Err(cancelled());
        }
        let mut sessions = self.sessions.lock().map_err(|_| unavailable())?;
        if sessions.contains_key(session_id) {
            worker.terminate()?;
            return Err(invalid_response());
        }
        sessions.insert(session_id.to_owned(), worker);
        Ok(())
    }

    fn speak(
        &self,
        session_id: &str,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        if text.trim().is_empty() || text.len() > MAX_PHRASE_BYTES {
            return Err(invalid_response());
        }
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let worker = self.worker(session_id)?;
        let result = worker.request(
            json!({"command": "speak", "text": text}),
            SPEECH_TIMEOUT,
            Some(cancellation),
            false,
        );
        if result.as_ref().err() == Some(&cancelled()) {
            // Turn cancellation can occur independently of the browser STOP.
            // The private sender must be interrupted even if the Rust caller
            // stopped waiting while its previously accepted utterance plays.
            if worker
                .request(
                    json!({"command": "interrupt"}),
                    INTERRUPT_TIMEOUT,
                    None,
                    true,
                )
                .is_err()
            {
                // Never leave an unconfirmed private audio publisher running.
                let _ = worker.terminate();
                return Err(unavailable());
            }
        }
        result
    }

    fn interrupt(&self, session_id: &str) -> Result<(), ProviderError> {
        let worker = self.worker(session_id)?;
        let result = worker.request(
            json!({"command": "interrupt"}),
            INTERRUPT_TIMEOUT,
            None,
            false,
        );
        if result.is_err() {
            // Preserve the unconfirmed STOP outcome for the later canonical
            // close. Do not fabricate success when the child has been killed.
            let _stop = worker.stop_gate.lock().map_err(|_| unavailable())?;
            let _ = worker.mark_stopped();
            let _ = worker.terminate();
            let mut receipt = worker.stop_receipt.lock().map_err(|_| unavailable())?;
            if receipt.is_none() {
                *receipt = Some(false);
            }
        }
        result
    }

    fn stop(&self, session_id: &str) -> Result<(), ProviderError> {
        let worker = self.worker(session_id)?;
        let _stop = worker.stop_gate.lock().map_err(|_| unavailable())?;
        if let Some(confirmed) = *worker.stop_receipt.lock().map_err(|_| unavailable())? {
            return if confirmed { Ok(()) } else { Err(unavailable()) };
        }
        worker.mark_stopped()?;
        // STOP must clear already queued avatar speech, not merely disconnect
        // the private publisher and leave D-ID playing its queued audio.
        let interrupted = worker.request(
            json!({"command": "interrupt"}),
            INTERRUPT_TIMEOUT,
            None,
            true,
        );
        // Even on timeout or a disconnected pipe, terminate the only private
        // publisher. Never return success when the interrupt receipt was absent.
        let terminated = worker.terminate();
        let success = interrupted.is_ok() && terminated.is_ok();
        *worker.stop_receipt.lock().map_err(|_| unavailable())? = Some(success);
        // Retain only the stop receipt until canonical close explicitly forgets
        // this publisher. A second STOP must reproduce the SAME result.
        interrupted.and(terminated)
    }

    fn forget(&self, session_id: &str) -> Result<(), ProviderError> {
        let worker = self.worker(session_id)?;
        let _stop = worker.stop_gate.lock().map_err(|_| unavailable())?;
        worker.terminate()?;
        self.sessions
            .lock()
            .map_err(|_| unavailable())?
            .remove(session_id);
        Ok(())
    }
}

impl Drop for EchoPythonBackend {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.get_mut() {
            for worker in sessions.values() {
                let _ = worker.terminate();
            }
            sessions.clear();
        }
    }
}

#[cfg(all(test, unix))]
mod bounded_stop_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicBool;

    struct NeverCancelled(AtomicBool);

    impl CancellationProbe for NeverCancelled {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }

    #[test]
    fn unresponsive_echo_interrupt_is_fenced_and_child_killed_without_five_second_wait() {
        // A real child OS process, not a mocked DidEchoBackend. The fake
        // interpreter acknowledges open, but indefinitely stalls on did.interrupt.
        // No LiveKit account, paid TTS call or real network access is needed.
        let script = std::env::temp_dir().join(format!(
            "vpr-echo-hung-{}-{}.sh",
            std::process::id(),
            std::thread::current().name().unwrap_or("stop")
        ));
        std::fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"command":"open"'*) printf '{"id":0,"ok":true}\n' ;;
    *'"command":"interrupt"'*) while :; do :; done ;;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();

        let configuration = EchoPythonConfig::new(
            script.to_string_lossy().into_owned(),
            "https://tts.example.test/v1/audio/speech",
            "test-only-key",
            "test-model",
            "test-voice",
        )
        .unwrap();
        let backend = EchoPythonBackend::new(configuration);
        let probe = NeverCancelled(AtomicBool::new(false));
        backend
            .open("hung-session", "wss://livekit.example.test/room/agent-1", "test-token", &probe)
            .unwrap();
        let started = Instant::now();
        let failed = backend.stop("hung-session");
        let elapsed = started.elapsed();
        let _ = std::fs::remove_file(&script);
        assert!(failed.is_err(), "vendor receipt missing: STOP must not be reported successful");
        assert!(
            elapsed < Duration::from_millis(1800),
            "STOP was blocked by a nonresponsive private worker: {elapsed:?}"
        );
        assert!(backend.stop("hung-session").is_err(), "unconfirmed STOP must remain unconfirmed");
        backend.forget("hung-session").unwrap();
        assert!(
            backend.worker("hung-session").is_err(),
            "dead publisher must be unregistered"
        );
    }
    #[test]
    fn acknowledged_echo_stop_is_reusable_for_canonical_close_without_replaying_control() {
        let script = std::env::temp_dir().join(format!(
            "vpr-echo-ack-{}.sh",
            std::process::id()
        ));
        std::fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"command":"open"'*) printf '{"id":0,"ok":true}\n' ;;
    *'"command":"interrupt"'*) printf '{"id":1,"ok":true}\n' ;;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();

        let backend = EchoPythonBackend::new(
            EchoPythonConfig::new(
                script.to_string_lossy().into_owned(),
                "https://tts.example.test/v1/audio/speech",
                "test-only-key",
                "test-model",
                "test-voice",
            )
            .unwrap(),
        );
        let probe = NeverCancelled(AtomicBool::new(false));
        backend
            .open("ack-session", "wss://livekit.example.test/room/agent-1", "token", &probe)
            .unwrap();
        backend.stop("ack-session").unwrap();
        backend.stop("ack-session").unwrap();
        assert!(
            backend.speak("ack-session", "late speech", &probe).is_err(),
            "STOP must fence all future egress"
        );
        backend.forget("ack-session").unwrap();
        let _ = std::fs::remove_file(&script);
        assert!(backend.worker("ack-session").is_err());
    }

}
