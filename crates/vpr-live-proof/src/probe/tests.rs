use std::sync::{Arc, Mutex};

use vpr_evaluation::{
    ProviderRole, ProviderStateBinding, ProviderStateManifest, RT0_PROVIDER_STATE_SCHEMA,
};
use vpr_integration::{
    CancellationProbe, GeneratedAudioSink, GeneratedTextSink, LlmPort, LlmRequest, PcmSampleFormat,
    ProviderDescriptor as PortDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, SttPort, SttRequest, Transcript, TtsPort, TtsRequest, UsageEvidence,
    UsageUnit, WebRtcIceCandidate, WebRtcSessionDescription,
};
use vpr_owner_lab::{ProviderBundle, ProviderDescriptor};

use super::{LiveProviderProbeError, RT0_LIVE_PROVIDER_PROBE_SCHEMA, run_provider_probe};
use crate::{LiveProofPreflightReceipt, PreparedLiveProof, RT0_LIVE_PROOF_PREFLIGHT_SCHEMA};

#[derive(Default)]
struct AvatarStats {
    create_calls: usize,
    close_calls: usize,
    fail_first_close: bool,
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
        assert_eq!(request.audio.channels, 1);
        Ok((
            Transcript {
                text: "Секретная тестовая транскрипция".into(),
                locale: "ru-RU".into(),
            },
            UsageEvidence {
                input_units: Some(100),
                input_unit: Some(UsageUnit::AudioMillisecond),
                ..UsageEvidence::default()
            },
        ))
    }
}

struct FakeLlm;

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
        assert!(!request.context.contains("Секретная тестовая транскрипция"));
        sink.push_generated_text("секретный-ответ")?;
        Ok(UsageEvidence {
            input_units: Some(8),
            input_unit: Some(UsageUnit::Token),
            output_units: Some(2),
            output_unit: Some(UsageUnit::Token),
            estimated_cost_microunits: Some(7),
            provider_charge_microunits: None,
        })
    }
}

struct FakeTts;

impl TtsPort for FakeTts {
    fn descriptor(&self) -> PortDescriptor {
        PortDescriptor {
            provider: "fake-tts".into(),
            model: "fake-tts-v1".into(),
            representation: Some("fake-voice".into()),
        }
    }

    fn synthesize(
        &self,
        request: &TtsRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError> {
        assert!(!cancellation.is_cancelled());
        assert_eq!(request.text, "Готов.");
        assert_eq!(request.locale_hint.as_deref(), Some("ru-RU"));
        sink.push_generated_audio(&vec![3_u8; 6_400], 16_000, 1, PcmSampleFormat::S16Le)?;
        Ok(UsageEvidence {
            input_units: Some(6),
            input_unit: Some(UsageUnit::TextCharacter),
            output_units: Some(200),
            output_unit: Some(UsageUnit::AudioMillisecond),
            estimated_cost_microunits: Some(5),
            provider_charge_microunits: None,
        })
    }
}

struct FakeAvatar {
    stats: Arc<Mutex<AvatarStats>>,
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
            provider_stream_id: "secret-stream-id".into(),
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
        _text: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
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

fn prepared(stats: Arc<Mutex<AvatarStats>>) -> PreparedLiveProof {
    let stt_descriptor = descriptor("fake-stt", "fake-stt-v1", 'a');
    let llm_descriptor = descriptor("fake-llm", "fake-llm-v1", 'b');
    let tts_descriptor = descriptor("fake-tts", "fake-tts-v1/fake-voice", 'd');
    let avatar_descriptor = descriptor("fake-avatar", "fake-avatar-v1", 'c');
    let provider_state = ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            state_binding(ProviderRole::Stt, &stt_descriptor),
            state_binding(ProviderRole::Llm, &llm_descriptor),
            state_binding(ProviderRole::Tts, &tts_descriptor),
            state_binding(ProviderRole::Avatar, &avatar_descriptor),
        ],
    };
    PreparedLiveProof {
        receipt: LiveProofPreflightReceipt {
            schema_version: RT0_LIVE_PROOF_PREFLIGHT_SCHEMA.into(),
            candidate_sha: "1".repeat(40),
            provider_state_sha256: "d".repeat(64),
            provider_state,
        },
        providers: ProviderBundle {
            avatar: Box::new(FakeAvatar { stats }),
            stt: Some(Box::new(FakeStt)),
            llm: Some(Box::new(FakeLlm)),
            tts: Some(Box::new(FakeTts)),
            avatar_descriptor,
            stt_descriptor: Some(stt_descriptor),
            llm_descriptor: Some(llm_descriptor),
            tts_descriptor: Some(tts_descriptor),
        },
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

#[test]
fn probe_uses_canonical_paths_and_serializes_only_sanitized_evidence() {
    let stats = Arc::new(Mutex::new(AvatarStats::default()));
    let receipt = run_provider_probe(prepared(Arc::clone(&stats)), vec![0; 3_200]).unwrap();
    assert_eq!(receipt.schema_version, RT0_LIVE_PROVIDER_PROBE_SCHEMA);
    assert!(!receipt.conversation_evidence);
    assert!(!receipt.output_delivery_proven);
    assert_eq!(receipt.input_audio_millis, 100);
    assert_eq!(receipt.input_audio_sha256.len(), 64);
    assert!(receipt.stt.transcript_chars > 0);
    assert!(receipt.llm.output_chars > 0);
    assert_eq!(receipt.tts.audio_millis, 200);
    assert_eq!(receipt.tts.audio_sha256.len(), 64);
    assert_eq!(receipt.tts.usage.output_units, Some(200));
    let json = serde_json::to_string(&receipt).unwrap();
    for secret in [
        "Секретная тестовая транскрипция",
        "секретный-ответ",
        "secret-stream-id",
        "secret-session-id",
        "secret-sdp-material",
    ] {
        assert!(!json.contains(secret));
    }
    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 1);
    assert_eq!(stats.close_calls, 1);
}

#[test]
fn invalid_audio_fails_before_any_provider_is_used() {
    let stats = Arc::new(Mutex::new(AvatarStats::default()));
    assert_eq!(
        run_provider_probe(prepared(Arc::clone(&stats)), vec![0; 3]),
        Err(LiveProviderProbeError::InvalidAudio)
    );
    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 0);
    assert_eq!(stats.close_calls, 0);
}

#[test]
fn avatar_close_failure_attempts_revoke_cleanup_and_remains_failure() {
    let stats = Arc::new(Mutex::new(AvatarStats {
        fail_first_close: true,
        ..AvatarStats::default()
    }));
    assert_eq!(
        run_provider_probe(prepared(Arc::clone(&stats)), vec![0; 3_200]),
        Err(LiveProviderProbeError::AvatarCleanup(
            vpr_domain::Rt0ReasonCode::ProviderUnavailable
        ))
    );
    let stats = stats.lock().unwrap();
    assert_eq!(stats.create_calls, 1);
    assert_eq!(stats.close_calls, 2);
}
