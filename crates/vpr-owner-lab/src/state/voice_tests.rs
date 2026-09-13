use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, ProviderDescriptor, ProviderError,
    ProviderErrorKind, RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, SttPort, SttRequest, Transcript, UsageEvidence, WebRtcIceServer,
    WebRtcSessionDescription,
};

use super::{LabError, OwnerLabEngine, OwnerLabStartRequest};

#[derive(Default)]
struct VoiceStats {
    stt: AtomicUsize,
    llm: AtomicUsize,
    avatar_text: AtomicUsize,
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
            provider_stream_id: "stream".into(),
            provider_session_id: "session".into(),
            offer: WebRtcSessionDescription {
                kind: "offer".into(),
                sdp: "v=0".into(),
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

fn sample_pcm() -> Vec<u8> {
    vec![0_u8; 3_200]
}

#[test]
fn voice_turn_runs_stt_llm_and_avatar_on_canonical_path() {
    let (mut engine, stats, _) = voice_engine(false);
    let result = engine.voice_turn(sample_pcm(), |_| {}).unwrap();
    assert_eq!(result.transcript, "Как дела?");
    assert_eq!(result.reply, "Всё хорошо.");
    assert_eq!(stats.stt.load(Ordering::SeqCst), 1);
    assert_eq!(stats.llm.load(Ordering::SeqCst), 1);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 1);
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
