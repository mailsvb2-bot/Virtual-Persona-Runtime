use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarSession, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription,
};

use super::*;

#[derive(Default)]
struct Stats {
    create: AtomicUsize,
    answer: AtomicUsize,
    ice: AtomicUsize,
    text: AtomicUsize,
    close: AtomicUsize,
    fail_create: AtomicUsize,
    fail_close: AtomicUsize,
}

struct FakeAvatar {
    stats: Arc<Stats>,
}

impl RealtimeAvatarPort for FakeAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "fake-avatar".into(),
            model: "fake-realtime".into(),
            representation: Some("owner".into()),
        }
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([RealtimeAvatarCapability::TextInput])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats.create.fetch_add(1, Ordering::SeqCst);
        if self.stats.fail_create.load(Ordering::SeqCst) > 0 {
            self.stats.fail_create.fetch_sub(1, Ordering::SeqCst);
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: true,
            });
        }
        Ok(RealtimeAvatarSession {
            provider_stream_id: "stream-secret".into(),
            provider_session_id: "session-secret".into(),
            offer: WebRtcSessionDescription {
                kind: "offer".into(),
                sdp: "v=0\r\n".into(),
            },
            ice_servers: vec![WebRtcIceServer {
                urls: vec!["stun:example.test".into()],
                username: None,
                credential: None,
            }],
        })
    }

    fn submit_answer(
        &self,
        _session: &RealtimeAvatarSession,
        _answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats.answer.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn submit_ice_candidate(
        &self,
        _session: &RealtimeAvatarSession,
        _candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats.ice.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn speak_text(
        &self,
        _session: &RealtimeAvatarSession,
        _text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats.text.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn speak_audio_url(
        &self,
        _session: &RealtimeAvatarSession,
        _audio_url: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    fn close_session(&self, _session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        self.stats.close.fetch_add(1, Ordering::SeqCst);
        if self.stats.fail_close.load(Ordering::SeqCst) > 0 {
            self.stats.fail_close.fetch_sub(1, Ordering::SeqCst);
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: true,
            });
        }
        Ok(())
    }
}

fn engine_with_failures(
    egress_enabled: bool,
    create_failures: usize,
    close_failures: usize,
) -> (OwnerLabEngine, Arc<Stats>) {
    let stats = Arc::new(Stats::default());
    stats.fail_create.store(create_failures, Ordering::SeqCst);
    stats.fail_close.store(close_failures, Ordering::SeqCst);
    let provider = FakeAvatar {
        stats: Arc::clone(&stats),
    };
    (
        OwnerLabEngine::new(Box::new(provider), egress_enabled).unwrap(),
        stats,
    )
}

fn engine(egress_enabled: bool) -> (OwnerLabEngine, Arc<Stats>) {
    engine_with_failures(egress_enabled, 0, 0)
}

#[test]
fn start_requires_process_egress_gate_and_explicit_consent() {
    let (mut disabled, disabled_stats) = engine(false);
    assert_eq!(
        disabled.start(OwnerLabStartRequest { consent: true }),
        Err(LabError::EgressDisabled)
    );
    assert_eq!(disabled_stats.create.load(Ordering::SeqCst), 0);

    let (mut no_consent, consent_stats) = engine(true);
    assert_eq!(
        no_consent.start(OwnerLabStartRequest { consent: false }),
        Err(LabError::ConsentRequired)
    );
    assert_eq!(consent_stats.create.load(Ordering::SeqCst), 0);
}

#[test]
fn browser_signaling_and_text_each_use_fresh_authorized_turns() {
    let (mut engine, stats) = engine(true);
    let bundle = engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    assert_eq!(bundle.offer.kind, "offer");
    assert_eq!(bundle.capabilities, vec!["text"]);

    engine
        .apply(OwnerLabTurnInput::Answer(WebRtcSessionDescription {
            kind: "answer".into(),
            sdp: "v=0 answer".into(),
        }))
        .unwrap();
    engine
        .apply(OwnerLabTurnInput::Ice(WebRtcIceCandidate {
            candidate: Some("candidate:1".into()),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        }))
        .unwrap();
    engine
        .apply(OwnerLabTurnInput::Text("Привет".into()))
        .unwrap();

    assert_eq!(stats.create.load(Ordering::SeqCst), 1);
    assert_eq!(stats.answer.load(Ordering::SeqCst), 1);
    assert_eq!(stats.ice.load(Ordering::SeqCst), 1);
    assert_eq!(stats.text.load(Ordering::SeqCst), 1);
}

#[test]
fn revoke_blocks_new_content_but_still_cleans_remote_avatar() {
    let (mut engine, stats) = engine(true);
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    engine.revoke().unwrap();

    assert_eq!(engine.status().session_state, "revoked");
    assert!(!engine.status().avatar_open);
    assert_eq!(stats.close.load(Ordering::SeqCst), 1);
    assert!(
        engine
            .apply(OwnerLabTurnInput::Text("late".into()))
            .is_err()
    );
    assert_eq!(stats.text.load(Ordering::SeqCst), 0);

    engine.close().unwrap();
    assert_eq!(engine.status().session_state, "closed");
}
#[test]
fn failed_provider_create_does_not_publish_a_poisoned_session() {
    let (mut engine, stats) = engine_with_failures(true, 1, 0);
    let first = engine.start(OwnerLabStartRequest { consent: true });
    assert!(matches!(
        first,
        Err(LabError::Provider(Rt0ReasonCode::ProviderUnavailable))
    ));
    assert_eq!(engine.status().session_state, "none");
    assert!(!engine.status().avatar_open);

    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    assert_eq!(engine.status().session_state, "active");
    assert!(engine.status().avatar_open);
    assert_eq!(stats.create.load(Ordering::SeqCst), 2);
}

#[test]
fn repeated_revoke_retries_failed_remote_cleanup_without_reauthorizing() {
    let (mut engine, stats) = engine_with_failures(true, 0, 1);
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();

    assert!(matches!(
        engine.revoke(),
        Err(LabError::Provider(Rt0ReasonCode::ProviderUnavailable))
    ));
    assert_eq!(engine.status().session_state, "revoked");
    assert!(engine.status().avatar_open);
    assert_eq!(stats.close.load(Ordering::SeqCst), 1);

    engine.revoke().unwrap();
    assert_eq!(engine.status().session_state, "revoked");
    assert!(!engine.status().avatar_open);
    assert_eq!(stats.close.load(Ordering::SeqCst), 2);
}
