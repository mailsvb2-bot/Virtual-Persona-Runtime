use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use vpr_domain::{
    ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord,
    PersonaId, PersonaIdentity, PersonaMode, PersonaProfile, PersonaVersion, SourceKind,
    VerificationState,
};
use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, ProviderDescriptor, ProviderError,
    ProviderErrorKind, RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, SttPort, SttRequest, Transcript, UsageEvidence,
    WebRtcIceServer, WebRtcSessionDescription,
};

use super::{
    LabError, LabSessionAudience, OwnerContextState, OwnerLabEngine, OwnerLabStartRequest,
    OwnerLabTurnInput,
};

#[derive(Default)]
struct VoiceStats {
    stt: AtomicUsize,
    llm: AtomicUsize,
    avatar_text: AtomicUsize,
    contexts: Mutex<Vec<String>>,
}

struct VoiceAvatar {
    stats: Arc<VoiceStats>,
}

impl RealtimeAvatarPort for VoiceAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("voice-avatar")
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([RealtimeAvatarCapability::TextInput])
    }

    fn create_session(
        &self,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Ok(RealtimeAvatarSession {
            provider_resource_id: "stream".into(),
            provider_session_id: "session".into(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: "offer".into(),
                    sdp: "v=0".into(),
                },
                ice_servers: vec![WebRtcIceServer {
                    urls: vec!["stun:example.test".into()],
                    username: None,
                    credential: None,
                }],
            },
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
        _candidate: &vpr_integration::WebRtcIceCandidate,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    fn speak_text(
        &self,
        _session: &RealtimeAvatarSession,
        _text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        self.stats.avatar_text.fetch_add(1, Ordering::SeqCst);
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
        Ok(())
    }
}

struct ImmediateStt {
    stats: Arc<VoiceStats>,
}

impl SttPort for ImmediateStt {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("voice-stt")
    }

    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        if cancellation.is_cancelled() || !request.audio.is_well_formed() {
            return Err(cancelled());
        }
        self.stats.stt.fetch_add(1, Ordering::SeqCst);
        Ok((
            Transcript {
                text: "Как дела?".into(),
                locale: "ru-RU".into(),
            },
            UsageEvidence::default(),
        ))
    }
}

struct VoiceLlm {
    stats: Arc<VoiceStats>,
    block_until_cancelled: bool,
    started: Option<mpsc::Sender<()>>,
}

impl LlmPort for VoiceLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("voice-llm")
    }

    fn stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.stats.llm.fetch_add(1, Ordering::SeqCst);
        self.stats
            .contexts
            .lock()
            .unwrap()
            .push(request.context.clone());
        if let Some(started) = &self.started {
            started.send(()).unwrap();
        }
        assert!(request.context.contains("Как дела?"));
        if self.block_until_cancelled {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancellation.is_cancelled() {
                assert!(
                    Instant::now() < deadline,
                    "voice LLM did not observe interrupt"
                );
                thread::yield_now();
            }
            return Err(cancelled());
        }
        sink.push_generated_text("Всё хорошо.")?;
        Ok(UsageEvidence::default())
    }
}

fn voice_engine(
    block_until_cancelled: bool,
) -> (OwnerLabEngine, Arc<VoiceStats>, Option<mpsc::Receiver<()>>) {
    let stats = Arc::new(VoiceStats::default());
    let avatar = VoiceAvatar {
        stats: Arc::clone(&stats),
    };
    let stt = ImmediateStt {
        stats: Arc::clone(&stats),
    };
    let (started, started_rx) = if block_until_cancelled {
        let (tx, rx) = mpsc::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let llm = VoiceLlm {
        stats: Arc::clone(&stats),
        block_until_cancelled,
        started,
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    (engine, stats, started_rx)
}

fn reviewed_profile() -> PersonaProfile {
    let mut profile = PersonaProfile::new(
        PersonaIdentity::new(
            PersonaId::new("reviewed-owner-context-persona").unwrap(),
            PersonaVersion::new(1).unwrap(),
            PersonaMode::DigitalTwin,
        ),
        ConstitutionBoundary::strict_digital_twin(),
    );
    let id = ClaimId::new("opinion-working-style").unwrap();
    profile
        .add_captured_claim(
            OwnerClaimRecord::capture(
                id.clone(),
                OwnerClaim {
                    statement: "Люблю быстрые итерации".into(),
                    kind: ClaimKind::Opinion,
                    source: SourceKind::Owner,
                    verification: VerificationState::Unverified,
                    derivation: DerivationKind::Direct,
                },
            )
            .unwrap(),
        )
        .unwrap();
    profile.mark_capture_complete().unwrap();
    profile.approve_claim(&id).unwrap();
    profile.approve_initial_review().unwrap();
    profile
}

fn reviewed_voice_engine() -> (OwnerLabEngine, Arc<VoiceStats>) {
    let stats = Arc::new(VoiceStats::default());
    let avatar = VoiceAvatar {
        stats: Arc::clone(&stats),
    };
    let stt = ImmediateStt {
        stats: Arc::clone(&stats),
    };
    let llm = VoiceLlm {
        stats: Arc::clone(&stats),
        block_until_cancelled: false,
        started: None,
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_reviewed_profile(reviewed_profile())
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    (engine, stats)
}

fn reviewed_visitor_voice_engine() -> (OwnerLabEngine, Arc<VoiceStats>) {
    let stats = Arc::new(VoiceStats::default());
    let avatar = VoiceAvatar {
        stats: Arc::clone(&stats),
    };
    let stt = ImmediateStt {
        stats: Arc::clone(&stats),
    };
    let llm = VoiceLlm {
        stats: Arc::clone(&stats),
        block_until_cancelled: false,
        started: None,
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_reviewed_profile(reviewed_profile())
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start_visitor(OwnerLabStartRequest { consent: true })
        .unwrap();
    (engine, stats)
}

fn sample_pcm() -> Vec<u8> {
    vec![0_u8; 3_200]
}

#[test]
fn voice_turn_runs_stt_llm_and_avatar_on_canonical_path() {
    let (mut engine, stats, _) = voice_engine(false);
    let result = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    assert_eq!(result.transcript, "Как дела?");
    assert_eq!(result.reply, "Всё хорошо.");
    assert!(result.evidence_turn_sequence > 0);
    assert!(result.evidence_output_sequence > 0);
    assert!(result.client_command.is_none());
    assert_eq!(
        engine.acknowledge_voice_playback(
            result.evidence_turn_sequence,
            result.evidence_output_sequence + 1,
        ),
        Err(LabError::InvalidState)
    );
    engine
        .acknowledge_voice_playback(
            result.evidence_turn_sequence,
            result.evidence_output_sequence,
        )
        .unwrap();
    engine
        .acknowledge_voice_playback(
            result.evidence_turn_sequence,
            result.evidence_output_sequence,
        )
        .unwrap();
    assert_eq!(stats.stt.load(Ordering::SeqCst), 1);
    assert_eq!(stats.llm.load(Ordering::SeqCst), 1);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 1);
}

#[test]
fn playback_ack_from_a_previous_session_cannot_cross_session_restart() {
    let (mut engine, _, _) = voice_engine(false);
    let first = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    engine.close().unwrap();
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    assert_eq!(
        engine.acknowledge_voice_playback(
            first.evidence_turn_sequence,
            first.evidence_output_sequence,
        ),
        Err(LabError::InvalidState)
    );
}

#[test]
fn corrected_reviewed_claim_is_used_by_the_next_canonical_voice_turn() {
    let (mut engine, stats) = reviewed_voice_engine();
    assert_eq!(
        engine.status().owner_context_state,
        OwnerContextState::Reviewed
    );
    assert_eq!(engine.status().persona_version, 2);
    assert_eq!(engine.status().reviewed_owner_claims, 1);

    engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    let id = ClaimId::new("opinion-working-style").unwrap();
    engine
        .correct_owner_claim(
            &id,
            "Предпочитаю короткие циклы проверки",
            ClaimKind::Opinion,
        )
        .unwrap();
    assert_eq!(engine.status().persona_version, 3);
    engine.voice_turn(sample_pcm(), |_| {}).unwrap();

    let contexts = stats.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    assert!(contexts[0].contains("[verified_owner_opinion] Люблю быстрые итерации"));
    assert!(contexts[1].contains("[verified_owner_opinion] Предпочитаю короткие циклы проверки"));
    assert!(!contexts[1].contains("Люблю быстрые итерации"));
}

#[test]
fn visitor_voice_turn_excludes_reviewed_owner_context_and_blocks_direct_speech() {
    let (mut engine, stats) = reviewed_visitor_voice_engine();
    assert_eq!(
        engine.status().session_audience,
        Some(LabSessionAudience::Visitor)
    );
    assert_eq!(engine.status().persona_version, 2);
    assert_eq!(engine.status().reviewed_owner_claims, 0);
    assert!(matches!(
        engine.reviewed_owner_context_snapshot(),
        Err(LabError::Runtime(
            vpr_domain::Rt0ReasonCode::AuthScopeDenied
        ))
    ));
    let owner_claim_id = ClaimId::new("opinion-working-style").unwrap();
    assert_eq!(
        engine.correct_owner_claim(
            &owner_claim_id,
            "Попытка visitor-перезаписи",
            ClaimKind::Opinion,
        ),
        Err(LabError::Runtime(
            vpr_domain::Rt0ReasonCode::AuthScopeDenied
        ))
    );

    engine.voice_turn(sample_pcm(), |_| {}).unwrap();

    let contexts = stats.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].contains("RT0 visitor-scoped conversation"));
    assert!(contexts[0].contains("visitor scope does not provide verified owner material"));
    assert!(!contexts[0].contains("Люблю быстрые итерации"));
    drop(contexts);

    assert_eq!(
        engine.apply(OwnerLabTurnInput::Text(
            "Скажи это от лица владельца".into()
        )),
        Err(LabError::Runtime(
            vpr_domain::Rt0ReasonCode::AuthScopeDenied
        ))
    );
}

#[test]
fn interrupt_handle_cancels_in_flight_voice_before_avatar_output() {
    let (mut engine, stats, llm_started) = voice_engine(true);
    let llm_started = llm_started.expect("blocking LLM must expose a start signal");
    let (handle_tx, handle_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        engine.voice_turn(sample_pcm(), |handle| {
            handle_tx.send(handle).unwrap();
        })
    });

    let handle = handle_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    llm_started.recv_timeout(Duration::from_secs(1)).unwrap();
    handle.interrupt().unwrap();
    let result = worker.join().unwrap();
    assert_eq!(
        result,
        Err(LabError::Runtime(vpr_domain::Rt0ReasonCode::TurnCancelled))
    );
    assert_eq!(stats.stt.load(Ordering::SeqCst), 1);
    assert_eq!(stats.llm.load(Ordering::SeqCst), 1);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 0);
}

fn descriptor(provider: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        provider: provider.into(),
        model: "contract".into(),
        representation: None,
    }
}

fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}
