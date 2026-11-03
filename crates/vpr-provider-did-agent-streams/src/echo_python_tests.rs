//! Native subprocess STOP / cleanup regressions; no cloud credentials.
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
fn unresponsive_echo_close_is_fenced_and_child_killed_without_five_second_wait() {
    // A real child OS process, not a mocked DidEchoBackend. The fake
    // interpreter acknowledges open, but indefinitely stalls on final close.
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
*'"command":"close"'*) while :; do :; done ;;
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
        .open(
            "hung-session",
            "wss://livekit.example.test/room/agent-1",
            "test-token",
            &probe,
        )
        .unwrap();
    let started = Instant::now();
    let failed = backend.stop("hung-session");
    let elapsed = started.elapsed();
    let _ = std::fs::remove_file(&script);
    assert!(
        failed.is_err(),
        "vendor receipt missing: STOP must not be reported successful"
    );
    assert!(
        elapsed < Duration::from_millis(1800),
        "STOP was blocked by a nonresponsive private worker: {elapsed:?}"
    );
    assert!(
        backend.stop("hung-session").is_err(),
        "unconfirmed STOP must remain unconfirmed"
    );
    backend.forget("hung-session").unwrap();
    assert!(
        backend.worker("hung-session").is_err(),
        "dead publisher must be unregistered"
    );
}
#[test]
fn acknowledged_echo_stop_is_reusable_for_canonical_close_without_replaying_control() {
    let script = std::env::temp_dir().join(format!("vpr-echo-ack-{}.sh", std::process::id()));
    std::fs::write(
        &script,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
*'"command":"open"'*) printf '{"id":0,"ok":true}\n' ;;
*'"command":"close"'*) printf '{"id":1,"ok":true}\n' ;;
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
        .open(
            "ack-session",
            "wss://livekit.example.test/room/agent-1",
            "token",
            &probe,
        )
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
