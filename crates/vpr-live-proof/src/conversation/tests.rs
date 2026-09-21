use std::sync::{Arc, Mutex};

use vpr_evaluation::{
    ProviderRole, ProviderStateBinding, ProviderStateManifest, RT0_PROVIDER_STATE_SCHEMA,
    sha256_hex,
};
use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest,
    ProviderDescriptor as PortDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, SttPort, SttRequest, Transcript, UsageEvidence, UsageUnit,
    WebRtcIceCandidate, WebRtcSessionDescription,
};
use vpr_owner_lab::{ProviderBundle, ProviderDescriptor};

use super::{
    LiveConversationAttemptError, ProofStatus, RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA,
    run_live_conversation_attempt,
};
use crate::{LiveProofPreflightReceipt, PreparedLiveProof, RT0_LIVE_PROOF_PREFLIGHT_SCHEMA};

const SECRET_CLAIM: &str = "Секретное мнение владельца";
const SECRET_TRANSCRIPT: &str = "Секретная тестовая транскрипция";
const SECRET_REPLY: &str = "секретный-ответ";

#[derive(Default)]
struct ConversationStats {
    create_calls: usize,
    close_calls: usize,
    fail_first_close: bool,
    llm_contexts: Vec<String>,
    spoken_texts: Vec<String>,
}

struct FakeStt;

impl SttPort for FakeStt {
    fn descriptor(&self) -> PortDescriptor {
        PortDescriptor {
            provider: "fake-stt".into(),
            model: "fake-stt-v1".into(),
            representation: None,
        }
    }

    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        assert!(!cancellation.is_cancelled());
        assert_eq!(request.audio.sample_rate_hz, 16_000);
        Ok((
            Transcript {
                text: SECRET_TRANSCRIPT.into(),
                locale: "ru-RU".into(),
            },
            UsageEvidence {
                input_units: Some(100),
                input_unit: Some(UsageUnit::AudioMillisecond),
                estimated_cost_microunits: Some(3),
                provider_charge_microunits: Some(4),
                ..UsageEvidence::default()
            },
        ))
    }
}

struct FakeLlm {
    stats: Arc<Mutex<ConversationStats>>,
}

impl LlmPort for FakeLlm {
    fn descriptor(&self) -> PortDescriptor {
        PortDescriptor {
            provider: "fake-llm".into(),
            model: "fake-llm-v1".into(),
            representation: None,
        }
    }

    fn stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats
            .lock()
            .unwrap()
            .llm_contexts
            .push(request.context.clone());
        sink.push_generated_text(SECRET_REPLY)?;
        Ok(UsageEvidence {
            input_units: Some(8),
            input_unit: Some(UsageUnit::Token),
            output_units: Some(2),
            output_unit: Some(UsageUnit::Token),
            estimated_cost_microunits: Some(7),
            provider_charge_microunits: Some(11),
        })
    }
}

struct FakeAvatar {
    stats: Arc<Mutex<ConversationStats>>,
}

impl RealtimeAvatarPort for FakeAvatar {
    fn descriptor(&self) -> PortDescriptor {
        PortDescriptor {
            provider: "fake-avatar".into(),
            model: "fake-avatar-v1".into(),
            representation: None,
        }
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::TextInput,
            RealtimeAvatarCapability::Interrupt,
        ])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.stats.lock().unwrap().create_calls += 1;
        Ok(RealtimeAvatarSession {
            provider_resource_id: "secret-stream-id".into(),
            provider_session_id: "secret-session-id".into(),
            offer: WebRtcSessionDescription {
                kind: "offer".into(),
                sdp: "secret-sdp-material".into(),
            },
            ice_servers: Vec::new(),
        })
    }

    fn submit_answer(
        &self,
        _session: &RealtimeAvatarSession,
        _answer: &WebRtcSessionDescription,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    fn submit_ice_candidate(
        &self,
        _session: &RealtimeAvatarSession,
        _candidate: &WebRtcIceCandidate,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    fn speak_text(
        &self,
        _session: &RealtimeAvatarSession,
        text: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.stats.lock().unwrap().spoken_texts.push(text.into());
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
        let mut stats = self.stats.lock().unwrap();
        stats.close_calls += 1;
        if stats.fail_first_close && stats.close_calls == 1 {
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: true,
            });
        }
        Ok(())
    }
}

fn descriptor(provider: &str, model: &str, fill: char) -> ProviderDescriptor {
    ProviderDescriptor {
        provider: provider.into(),
        model_or_representation: model.into(),
        configuration_fingerprint_sha256: fill.to_string().repeat(64),
    }
}

fn state_binding(role: ProviderRole, descriptor: &ProviderDescriptor) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: descriptor.provider.clone(),
        model_or_representation: descriptor.model_or_representation.clone(),
        configuration_fingerprint_sha256: descriptor.configuration_fingerprint_sha256.clone(),
    }
}

fn prepared(stats: Arc<Mutex<ConversationStats>>) -> PreparedLiveProof {
    let stt_descriptor = descriptor("fake-stt", "fake-stt-v1", 'a');
    let llm_descriptor = descriptor("fake-llm", "fake-llm-v1", 'b');
    let avatar_descriptor = descriptor("fake-avatar", "fake-avatar-v1", 'c');
    let provider_state = ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            state_binding(ProviderRole::Stt, &stt_descriptor),
            state_binding(ProviderRole::Llm, &llm_descriptor),
            state_binding(ProviderRole::Avatar, &avatar_descriptor),
        ],
    };
    let provider_state_sha256 = sha256_hex(&serde_json::to_vec_pretty(&provider_state).unwrap());
    PreparedLiveProof {
        receipt: LiveProofPreflightReceipt {
            schema_version: RT0_LIVE_PROOF_PREFLIGHT_SCHEMA.into(),
            candidate_sha: "1".repeat(40),
            provider_state_sha256,
            provider_state,
        },
        providers: ProviderBundle {
            avatar: Box::new(FakeAvatar {
                stats: Arc::clone(&stats),
            }),
            stt: Some(Box::new(FakeStt)),
            llm: Some(Box::new(FakeLlm { stats })),
            avatar_descriptor,
            stt_descriptor: Some(stt_descriptor),
            llm_descriptor: Some(llm_descriptor),
        },
    }
}

fn profile_bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version":"rt0-live-conversation-profile-0.1",
        "persona_id":"private-live-persona",
        "owner_review_confirmed":true,
        "claims":[{
            "claim_id":"opinion-1",
            "statement":SECRET_CLAIM,
            "kind":"opinion",
            "owner_approved":true
        }]
    }))
    .unwrap()
}

#[test]
fn owner_and_visitor_share_persona_but_not_private_owner_context() {
    let stats = Arc::new(Mutex::new(ConversationStats::default()));
    let profile = profile_bytes();
    let receipt = run_live_conversation_attempt(
        prepared(Arc::clone(&stats)),
        &profile,
        vec![1; 3_200],
        vec![2; 3_200],
    )
    .unwrap();

    assert_eq!(receipt.schema_version, RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA);
    assert_eq!(receipt.persona_version, 2);
    assert_eq!(receipt.reviewed_claims, 1);
    assert!(receipt.conversation_attempted);
    assert!(receipt.provider_output_submitted);
    assert_eq!(receipt.browser_media_playback, ProofStatus::NotProven);
    assert_eq!(receipt.video_render, ProofStatus::NotProven);
    assert_eq!(receipt.human_review, ProofStatus::NotProven);
    assert_eq!(receipt.owner.estimated_cost_microunits, Some(10));
    assert_eq!(receipt.visitor.estimated_cost_microunits, Some(10));
    assert_eq!(receipt.owner.provider_charge_microunits, Some(15));

    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 2);
    assert_eq!(stats.close_calls, 2);
    assert_eq!(stats.spoken_texts, vec![SECRET_REPLY, SECRET_REPLY]);
    assert_eq!(stats.llm_contexts.len(), 2);
    assert!(stats.llm_contexts[0].contains(SECRET_CLAIM));
    assert!(!stats.llm_contexts[1].contains(SECRET_CLAIM));
    assert!(stats.llm_contexts[1].contains("visitor-scoped"));

    let serialized = serde_json::to_string(&receipt).unwrap();
    for private_value in [
        SECRET_CLAIM,
        SECRET_TRANSCRIPT,
        SECRET_REPLY,
        "private-live-persona",
        "secret-stream-id",
        "secret-session-id",
        "secret-sdp-material",
    ] {
        assert!(!serialized.contains(private_value));
    }
}

#[test]
fn invalid_profile_fails_before_any_provider_session() {
    let stats = Arc::new(Mutex::new(ConversationStats::default()));
    let invalid =
        br#"{"schema_version":"wrong","persona_id":"p","owner_review_confirmed":true,"claims":[]}"#;
    assert_eq!(
        run_live_conversation_attempt(
            prepared(Arc::clone(&stats)),
            invalid,
            vec![1; 3_200],
            vec![2; 3_200],
        ),
        Err(LiveConversationAttemptError::InvalidProfile)
    );
    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 0);
    assert_eq!(stats.close_calls, 0);
}

#[test]
fn invalid_audio_fails_before_profile_or_provider_use() {
    let stats = Arc::new(Mutex::new(ConversationStats::default()));
    assert_eq!(
        run_live_conversation_attempt(
            prepared(Arc::clone(&stats)),
            b"not-json",
            vec![1; 3],
            vec![2; 3_200],
        ),
        Err(LiveConversationAttemptError::InvalidAudio)
    );
    assert_eq!(stats.lock().unwrap().create_calls, 0);
}

#[test]
fn owner_cleanup_failure_stops_before_visitor_session() {
    let stats = Arc::new(Mutex::new(ConversationStats {
        fail_first_close: true,
        ..ConversationStats::default()
    }));
    assert_eq!(
        run_live_conversation_attempt(
            prepared(Arc::clone(&stats)),
            &profile_bytes(),
            vec![1; 3_200],
            vec![2; 3_200],
        ),
        Err(LiveConversationAttemptError::OwnerCleanupFailed)
    );
    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 1);
    assert_eq!(stats.close_calls, 2);
}

#[test]
fn explicit_owner_review_flags_are_required_before_provider_use() {
    let stats = Arc::new(Mutex::new(ConversationStats::default()));
    let profile = serde_json::to_vec(&serde_json::json!({
        "schema_version":"rt0-live-conversation-profile-0.1",
        "persona_id":"private-live-persona",
        "owner_review_confirmed":false,
        "claims":[{
            "claim_id":"opinion-1",
            "statement":SECRET_CLAIM,
            "kind":"opinion",
            "owner_approved":true
        }]
    }))
    .unwrap();
    assert_eq!(
        run_live_conversation_attempt(
            prepared(Arc::clone(&stats)),
            &profile,
            vec![1; 3_200],
            vec![2; 3_200],
        ),
        Err(LiveConversationAttemptError::InvalidProfile)
    );
    assert_eq!(stats.lock().unwrap().create_calls, 0);
}

#[test]
fn every_claim_requires_explicit_owner_approval_before_provider_use() {
    let stats = Arc::new(Mutex::new(ConversationStats::default()));
    let profile = serde_json::to_vec(&serde_json::json!({
        "schema_version":"rt0-live-conversation-profile-0.1",
        "persona_id":"private-live-persona",
        "owner_review_confirmed":true,
        "claims":[{
            "claim_id":"opinion-1",
            "statement":SECRET_CLAIM,
            "kind":"opinion",
            "owner_approved":false
        }]
    }))
    .unwrap();
    assert_eq!(
        run_live_conversation_attempt(
            prepared(Arc::clone(&stats)),
            &profile,
            vec![1; 3_200],
            vec![2; 3_200],
        ),
        Err(LiveConversationAttemptError::InvalidProfile)
    );
    assert_eq!(stats.lock().unwrap().create_calls, 0);
}
