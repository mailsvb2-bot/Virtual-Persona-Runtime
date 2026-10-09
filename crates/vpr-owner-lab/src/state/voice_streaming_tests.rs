use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, LlmTextStream, ProviderDescriptor,
    ProviderError, ProviderErrorKind, RealtimeAvatarCapabilities, RealtimeAvatarCapability,
    RealtimeAvatarClientCommand, RealtimeAvatarClientControl, RealtimeAvatarClientRoute,
    RealtimeAvatarPort, RealtimeAvatarSession, RealtimeAvatarTransport, SttAudioStream, SttPort,
    SttRequest, SttStreamEvent, SttStreamRequest, Transcript, UsageEvidence, WebRtcIceServer,
    WebRtcSessionDescription,
};

use super::{LabError, OwnerLabEngine, OwnerLabStartRequest};

#[derive(Default)]
struct StreamingStats {
    stt_stream: AtomicUsize,
    stt_batch: AtomicUsize,
    llm: AtomicUsize,
    avatar_text: AtomicUsize,
}

struct StreamingAvatar {
    stats: Arc<StreamingStats>,
    client_text: bool,
}

impl RealtimeAvatarPort for StreamingAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("streaming-avatar")
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([RealtimeAvatarCapability::TextInput])
    }

    fn create_session(
        &self,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        let transport = if self.client_text {
            RealtimeAvatarTransport::LiveKit {
                server_url: "wss://livekit.example.test".into(),
                token: "session-token".into(),
            }
        } else {
            RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: "offer".into(),
                    sdp: "v=0".into(),
                },
                ice_servers: vec![WebRtcIceServer {
                    urls: vec!["stun:example.test".into()],
                    username: None,
                    credential: None,
                }],
            }
        };
        Ok(RealtimeAvatarSession {
            provider_resource_id: "stream".into(),
            provider_session_id: "session".into(),
            transport,
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

    fn client_control(
        &self,
        _session: &RealtimeAvatarSession,
    ) -> Option<RealtimeAvatarClientControl> {
        self.client_text.then_some(RealtimeAvatarClientControl {
            event_route: None,
            interrupt: true,
            interrupt_requires_playback_id: false,
            text_input: true,
        })
    }

    fn prepare_client_text(
        &self,
        _session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        if !self.client_text || cancellation.is_cancelled() || text.trim().is_empty() {
            return Err(cancelled());
        }
        self.stats.avatar_text.fetch_add(1, Ordering::SeqCst);
        Ok(RealtimeAvatarClientCommand {
            route: RealtimeAvatarClientRoute::LiveKitTextTopic {
                topic: "test.speak".into(),
            },
            payload: text.to_owned(),
        })
    }

    fn prepare_client_interrupt(
        &self,
        _session: &RealtimeAvatarSession,
        _playback_id: Option<&str>,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        if !self.client_text || cancellation.is_cancelled() {
            return Err(cancelled());
        }
        Ok(RealtimeAvatarClientCommand {
            route: RealtimeAvatarClientRoute::LiveKitTextTopic {
                topic: "test.interrupt".into(),
            },
            payload: "{}".into(),
        })
    }

    fn close_session(&self, _session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        Ok(())
    }
}

struct StreamingStt {
    stats: Arc<StreamingStats>,
    emit_final: bool,
}

struct StreamingSttSession {
    phase: u8,
    finished: bool,
    bytes: usize,
    emit_final: bool,
}

impl SttAudioStream for StreamingSttSession {
    fn push_audio(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() || self.finished || pcm.is_empty() || pcm.len() % 2 != 0 {
            return Err(cancelled());
        }
        self.bytes = self.bytes.saturating_add(pcm.len());
        Ok(())
    }

    fn finish_input(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() || self.bytes == 0 {
            return Err(cancelled());
        }
        self.finished = true;
        Ok(())
    }

    fn next_event(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<SttStreamEvent>, ProviderError> {
        if cancellation.is_cancelled() || !self.finished {
            return Err(cancelled());
        }
        let event = match self.phase {
            0 => Some(SttStreamEvent::Interim(Transcript {
                text: "Как".into(),
                locale: "ru-RU".into(),
            })),
            1 if self.emit_final => Some(SttStreamEvent::Final(Transcript {
                text: "Как дела?".into(),
                locale: "ru-RU".into(),
            })),
            _ => None,
        };
        self.phase = self.phase.saturating_add(1);
        Ok(event)
    }

    fn usage(&self) -> UsageEvidence {
        UsageEvidence::default()
    }
}

impl SttPort for StreamingStt {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("streaming-stt")
    }

    fn open_stream(
        &self,
        request: &SttStreamRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn SttAudioStream>, ProviderError> {
        if cancellation.is_cancelled() || !request.is_well_formed() {
            return Err(cancelled());
        }
        self.stats.stt_stream.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(StreamingSttSession {
            phase: 0,
            finished: false,
            bytes: 0,
            emit_final: self.emit_final,
        }))
    }

    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        if cancellation.is_cancelled() || !request.audio.is_well_formed() {
            return Err(cancelled());
        }
        self.stats.stt_batch.fetch_add(1, Ordering::SeqCst);
        Ok((
            Transcript {
                text: "batch fallback".into(),
                locale: "ru-RU".into(),
            },
            UsageEvidence::default(),
        ))
    }
}

struct StreamingVoiceLlm {
    stats: Arc<StreamingStats>,
    release_tail: Arc<AtomicBool>,
}

struct StreamingVoiceStream {
    phase: u8,
    release_tail: Arc<AtomicBool>,
}

impl LlmTextStream for StreamingVoiceStream {
    fn next_chunk(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<String>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        match self.phase {
            0 => {
                self.phase = 1;
                Ok(Some("Первая фраза. ".into()))
            }
            1 => {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !self.release_tail.load(Ordering::SeqCst) {
                    if cancellation.is_cancelled() {
                        return Err(cancelled());
                    }
                    assert!(
                        Instant::now() < deadline,
                        "streaming LLM tail was never released"
                    );
                    thread::yield_now();
                }
                self.phase = 2;
                Ok(Some("Вторая фраза.".into()))
            }
            _ => Ok(None),
        }
    }

    fn usage(&self) -> UsageEvidence {
        UsageEvidence::default()
    }
}

impl LlmPort for StreamingVoiceLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor("streaming-voice-llm")
    }

    fn open_stream(
        &self,
        request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn LlmTextStream>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        self.stats.llm.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.user_input, "Как дела?");
        Ok(Box::new(StreamingVoiceStream {
            phase: 0,
            release_tail: Arc::clone(&self.release_tail),
        }))
    }

    fn stream(
        &self,
        _request: &LlmRequest,
        _cancellation: &dyn CancellationProbe,
        _sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }
}

fn client_text_streaming_voice_engine() -> (OwnerLabEngine, Arc<StreamingStats>, Arc<AtomicBool>) {
    let stats = Arc::new(StreamingStats::default());
    let release_tail = Arc::new(AtomicBool::new(false));
    let avatar = StreamingAvatar {
        stats: Arc::clone(&stats),
        client_text: true,
    };
    let stt = StreamingStt {
        stats: Arc::clone(&stats),
        emit_final: true,
    };
    let llm = StreamingVoiceLlm {
        stats: Arc::clone(&stats),
        release_tail: Arc::clone(&release_tail),
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    (engine, stats, release_tail)
}

fn streaming_voice_engine() -> (OwnerLabEngine, Arc<StreamingStats>, Arc<AtomicBool>) {
    let stats = Arc::new(StreamingStats::default());
    let release_tail = Arc::new(AtomicBool::new(false));
    let avatar = StreamingAvatar {
        stats: Arc::clone(&stats),
        client_text: false,
    };
    let stt = StreamingStt {
        stats: Arc::clone(&stats),
        emit_final: true,
    };
    let llm = StreamingVoiceLlm {
        stats: Arc::clone(&stats),
        release_tail: Arc::clone(&release_tail),
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();
    (engine, stats, release_tail)
}

fn sample_pcm() -> Vec<u8> {
    vec![0_u8; 3_200]
}

#[test]
fn streaming_voice_emits_first_phrase_before_llm_tail_completes() {
    let (mut engine, stats, release_tail) = streaming_voice_engine();
    let playback = engine.voice_playback_registry();
    let (segment_tx, segment_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = engine.voice_turn_streaming(
            sample_pcm(),
            |_| {},
            |segment| {
                segment_tx.send(segment).unwrap();
                Ok(())
            },
        );
        result_tx.send(result).unwrap();
    });

    let first = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(first.evidence_output_sequence > 0);
    playback
        .acknowledge_voice_delivery_sent(
            first.evidence_turn_sequence,
            first.evidence_output_sequence,
        )
        .unwrap();
    playback
        .acknowledge_voice_playback(first.evidence_turn_sequence, first.evidence_output_sequence)
        .unwrap();
    assert!(matches!(
        result_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 1);

    release_tail.store(true, Ordering::SeqCst);
    let result = result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert_eq!(result.reply, "Первая фраза. Вторая фраза.");
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 2);
    assert_eq!(stats.stt_stream.load(Ordering::SeqCst), 1);
    assert_eq!(stats.stt_batch.load(Ordering::SeqCst), 0);
    assert_eq!(stats.llm.load(Ordering::SeqCst), 1);
}

#[test]
fn livekit_client_text_streams_first_phrase_before_generation_finishes() {
    let (mut engine, stats, release_tail) = client_text_streaming_voice_engine();
    let playback = engine.voice_playback_registry();
    let (segment_tx, segment_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = engine.voice_turn_streaming(
            sample_pcm(),
            |_| {},
            |segment| {
                segment_tx.send(segment).unwrap();
                Ok(())
            },
        );
        result_tx.send(result).unwrap();
    });

    let first = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(first.evidence_output_sequence > 0);
    let first_command = first.client_command.expect("first LiveKit command");
    assert_eq!(first_command.payload, "Первая фраза.");
    playback
        .acknowledge_voice_delivery_sent(
            first.evidence_turn_sequence,
            first.evidence_output_sequence,
        )
        .unwrap();
    assert!(matches!(
        result_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 1);

    release_tail.store(true, Ordering::SeqCst);
    let second = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(second.evidence_output_sequence > first.evidence_output_sequence);
    let second_command = second.client_command.expect("second LiveKit command");
    assert_eq!(second_command.payload, "Вторая фраза.");
    playback
        .acknowledge_voice_delivery_sent(
            second.evidence_turn_sequence,
            second.evidence_output_sequence,
        )
        .unwrap();
    playback
        .acknowledge_voice_playback_complete(
            first.evidence_turn_sequence,
            first.evidence_output_sequence,
        )
        .unwrap();
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 2);

    let result = result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(result.reply, "Первая фраза. Вторая фраза.");
    worker.join().unwrap();
}

#[test]
fn streaming_voice_without_final_stt_transcript_is_diagnostic_not_internal_error() {
    let stats = Arc::new(StreamingStats::default());
    let avatar = StreamingAvatar {
        stats: Arc::clone(&stats),
        client_text: false,
    };
    let stt = StreamingStt {
        stats: Arc::clone(&stats),
        emit_final: false,
    };
    let release_tail = Arc::new(AtomicBool::new(true));
    let llm = StreamingVoiceLlm {
        stats: Arc::clone(&stats),
        release_tail,
    };
    let mut engine = OwnerLabEngine::new(Box::new(avatar), true)
        .unwrap()
        .with_voice(Box::new(stt), Box::new(llm));
    engine
        .start(OwnerLabStartRequest { consent: true })
        .unwrap();

    assert_eq!(
        engine.voice_turn_streaming(sample_pcm(), |_| {}, |_| Ok(())),
        Err(LabError::SpeechNotRecognized)
    );
    assert_eq!(stats.stt_stream.load(Ordering::SeqCst), 1);
    assert_eq!(stats.llm.load(Ordering::SeqCst), 0);
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 0);
}

#[test]
fn streaming_voice_interrupt_cancels_remaining_llm_tail_after_first_segment() {
    let (mut engine, stats, _release_tail) = streaming_voice_engine();
    let (handle_tx, handle_rx) = mpsc::channel();
    let (segment_tx, segment_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        engine.voice_turn_streaming(
            sample_pcm(),
            |handle| handle_tx.send(handle).unwrap(),
            |segment| {
                segment_tx.send(segment).unwrap();
                Ok(())
            },
        )
    });

    let handle = handle_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let first = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(first.evidence_output_sequence > 0);
    handle.interrupt().unwrap();

    assert_eq!(
        worker.join().unwrap(),
        Err(LabError::Runtime(vpr_domain::Rt0ReasonCode::TurnCancelled))
    );
    assert_eq!(stats.avatar_text.load(Ordering::SeqCst), 1);
    assert!(matches!(
        segment_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
    ));
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
