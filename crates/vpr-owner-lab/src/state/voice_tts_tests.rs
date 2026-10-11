use super::*;
struct TestCanonicalTts {
    stats: Arc<VoiceStats>,
    qwen: bool,
}

impl TtsPort for TestCanonicalTts {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(if self.qwen {
            "metrotrance-qwen"
        } else {
            "metrotrance-chatterbox"
        })
    }

    fn synthesize(
        &self,
        _request: &TtsRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if self.qwen {
            self.stats.tts_qwen.fetch_add(1, Ordering::SeqCst);
        } else {
            self.stats.tts_chatterbox.fetch_add(1, Ordering::SeqCst);
        }
        sink.push_generated_audio(&[0_u8; 320], 16_000, 1, PcmSampleFormat::S16Le)?;
        Ok(UsageEvidence::default())
    }
}


#[test]
fn interrupted_reply_resume_uses_same_tts_pcm_owner_and_no_browser_did_speak() {
    let (engine, stats, _) = voice_engine(false);
    let mut engine = engine.with_named_tts(
        "piper".to_owned(),
        Box::new(TestCanonicalTts {
            stats: Arc::clone(&stats),
            qwen: true,
        }),
    );
    let original = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    assert!(original.client_command.is_none());
    let pcm_before = stats.avatar_pcm.load(Ordering::SeqCst);
    let tts_before = stats.tts_qwen.load(Ordering::SeqCst);
    let result = engine.prepare_resumed_speech("Продолжаю ответ.").unwrap();
    assert!(result.client_command.is_none(), "Echo never creates browser did.speak");
    assert_eq!(stats.avatar_pcm.load(Ordering::SeqCst), pcm_before + 1);
    assert_eq!(stats.tts_qwen.load(Ordering::SeqCst), tts_before + 1);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 0);
    assert!(result.evidence_turn_sequence > original.evidence_turn_sequence);
    engine.revoke_authority().unwrap();
    assert!(engine.prepare_resumed_speech("После отзыва.").is_err());
    assert_eq!(stats.avatar_pcm.load(Ordering::SeqCst), pcm_before + 1);
}

#[test]
fn voice_turn_switches_canonical_tts_both_ways_without_new_avatar_session() {
    let (engine, stats, _) = voice_engine(false);
    let mut engine = engine
        .with_named_tts(
            "metrotrance-qwen".to_owned(),
            Box::new(TestCanonicalTts {
                stats: Arc::clone(&stats),
                qwen: true,
            }),
        )
        .with_tts_fallback(
            "metrotrance-chatterbox".to_owned(),
            Box::new(TestCanonicalTts {
                stats: Arc::clone(&stats),
                qwen: false,
            }),
        );
    let first = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    engine
        .acknowledge_voice_playback(first.evidence_turn_sequence, first.evidence_output_sequence)
        .unwrap();
    engine
        .select_tts_provider("metrotrance-chatterbox")
        .unwrap();
    let second = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    engine
        .acknowledge_voice_playback(
            second.evidence_turn_sequence,
            second.evidence_output_sequence,
        )
        .unwrap();
    engine.select_tts_provider("metrotrance-qwen").unwrap();
    let third = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    engine
        .acknowledge_voice_playback(third.evidence_turn_sequence, third.evidence_output_sequence)
        .unwrap();
    assert_eq!(stats.tts_qwen.load(Ordering::SeqCst), 2);
    assert_eq!(stats.tts_chatterbox.load(Ordering::SeqCst), 1);
    assert_eq!(stats.avatar_pcm.load(Ordering::SeqCst), 3);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 0);
    assert_eq!(stats.avatar_creates.load(Ordering::SeqCst), 1);
}

struct BlockingCanonicalTts {
    started: mpsc::Sender<()>,
}

impl TtsPort for BlockingCanonicalTts {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("blocking-canonical-tts")
    }

    fn synthesize(
        &self,
        _request: &TtsRequest,
        cancellation: &dyn CancellationProbe,
        _sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.started.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: false,
        })
    }
}

#[test]
fn interrupt_during_canonical_tts_never_publishes_pcm_to_avatar() {
    let (engine, stats, _) = voice_engine(false);
    let (tts_started_tx, tts_started_rx) = mpsc::channel();
    let mut engine = engine.with_named_tts(
        "metrotrance-qwen".to_owned(),
        Box::new(BlockingCanonicalTts {
            started: tts_started_tx,
        }),
    );
    let (handle_tx, handle_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        engine.voice_turn(sample_pcm(), |handle| {
            handle_tx.send(handle).unwrap();
        })
    });
    let interrupt = handle_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    tts_started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    interrupt.interrupt().unwrap();
    let result = worker.join().unwrap();
    assert_eq!(
        result,
        Err(LabError::Runtime(vpr_domain::Rt0ReasonCode::TurnCancelled))
    );
    assert_eq!(stats.avatar_pcm.load(Ordering::SeqCst), 0);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 0);
}
