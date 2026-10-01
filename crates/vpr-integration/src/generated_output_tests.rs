use super::*;

#[test]
fn pcm_audio_reports_provider_neutral_duration() {
    let audio = AudioInput {
        pcm: vec![0; 640],
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
    };
    assert!(audio.is_well_formed());
    assert_eq!(audio.duration_millis(), Some(20));
}

#[test]
fn pcm_audio_rejects_partial_frames() {
    let audio = AudioInput {
        pcm: vec![0; 3],
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
    };
    assert!(!audio.is_well_formed());
    assert_eq!(audio.duration_millis(), None);
}

#[test]
fn streaming_stt_request_validates_chunk_alignment_without_owning_audio() {
    let request = SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    };
    assert!(request.is_well_formed());
    assert!(request.is_well_formed_chunk(&[0, 0, 1, 0]));
    assert!(!request.is_well_formed_chunk(&[]));
    assert!(!request.is_well_formed_chunk(&[0]));
}

#[test]
fn streaming_stt_event_preserves_interim_and_final_semantics() {
    let interim = SttStreamEvent::Interim(Transcript {
        text: "При".to_owned(),
        locale: "ru".to_owned(),
    });
    let final_event = SttStreamEvent::Final(Transcript {
        text: "Привет".to_owned(),
        locale: "ru".to_owned(),
    });
    assert!(!interim.is_final());
    assert!(final_event.is_final());
    assert_eq!(interim.transcript().text, "При");
    assert_eq!(final_event.transcript().text, "Привет");
}

#[test]
fn generated_text_buffer_accumulates_without_transport_contract() {
    let mut buffer = GeneratedTextBuffer::default();
    buffer.push_generated_text("При").unwrap();
    buffer.push_generated_text("вет").unwrap();
    assert_eq!(buffer.as_str(), "Привет");
}

#[test]
fn timed_generated_text_marks_only_first_meaningful_chunk() {
    let mut buffer = TimedGeneratedTextBuffer::start();
    buffer.push_generated_text("   ").unwrap();
    assert_eq!(buffer.first_meaningful_elapsed_millis(), None);
    buffer.push_generated_text("Привет").unwrap();
    let first = buffer.first_meaningful_elapsed_millis().unwrap();
    buffer.push_generated_text("!").unwrap();
    assert_eq!(buffer.as_str(), "   Привет!");
    assert_eq!(buffer.first_meaningful_elapsed_millis(), Some(first));
}

#[test]
fn generated_audio_buffer_rejects_sample_rate_changes() {
    let mut buffer = GeneratedAudioBuffer::default();
    buffer
        .push_generated_audio(&[1, 2], 16_000, 1, PcmSampleFormat::S16Le)
        .unwrap();
    let error = buffer
        .push_generated_audio(&[3, 4], 24_000, 1, PcmSampleFormat::S16Le)
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(buffer.pcm(), &[1, 2]);
    assert_eq!(buffer.sample_rate_hz(), Some(16_000));
    assert_eq!(buffer.channels(), Some(1));
    assert_eq!(buffer.sample_format(), Some(PcmSampleFormat::S16Le));
}

#[test]
fn generated_video_buffer_retains_generation_evidence_only() {
    let mut buffer = GeneratedVideoBuffer::default();
    buffer.push_generated_frame(&[7, 8], 42).unwrap();
    assert_eq!(buffer.frames().len(), 1);
    assert_eq!(buffer.frames()[0].encoded_frame, vec![7, 8]);
    assert_eq!(buffer.frames()[0].timestamp_micros, 42);
}

#[test]
fn media_debug_output_redacts_raw_payload_bytes() {
    let audio = AudioInput {
        pcm: vec![222, 173, 190, 239],
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
    };
    let mut generated_audio = GeneratedAudioBuffer::default();
    generated_audio
        .push_generated_audio(&[222, 173, 190, 239], 16_000, 1, PcmSampleFormat::S16Le)
        .unwrap();
    let mut generated_video = GeneratedVideoBuffer::default();
    generated_video
        .push_generated_frame(&[222, 173, 190, 239], 42)
        .unwrap();

    for debug in [
        format!("{audio:?}"),
        format!("{generated_audio:?}"),
        format!("{:?}", generated_video.frames()[0]),
        format!("{generated_video:?}"),
    ] {
        assert!(!debug.contains("222"));
        assert!(!debug.contains("173"));
        assert!(!debug.contains("190"));
        assert!(!debug.contains("239"));
    }
}

#[test]
fn generated_video_buffer_rejects_empty_or_decreasing_frames() {
    let mut buffer = GeneratedVideoBuffer::default();
    assert_eq!(
        buffer.push_generated_frame(&[], 1).unwrap_err().kind,
        ProviderErrorKind::InvalidResponse
    );
    buffer.push_generated_frame(&[1], 20).unwrap();
    assert_eq!(
        buffer.push_generated_frame(&[2], 19).unwrap_err().kind,
        ProviderErrorKind::InvalidResponse
    );
    assert_eq!(buffer.frames().len(), 1);
}
#[test]
fn generated_text_buffer_accepts_exact_limit_and_rejects_overflow_without_growth() {
    let mut buffer = GeneratedTextBuffer::default();
    let exact = "x".repeat(MAX_GENERATED_TEXT_BYTES);
    buffer.push_generated_text(&exact).unwrap();
    assert_eq!(buffer.as_str().len(), MAX_GENERATED_TEXT_BYTES);

    let error = buffer.push_generated_text("y").unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(buffer.as_str().len(), MAX_GENERATED_TEXT_BYTES);
}

#[test]
fn timed_generated_text_rejects_repeated_small_chunks_at_total_limit() {
    let mut buffer = TimedGeneratedTextBuffer::start();
    let chunk = "z".repeat(1024);
    for _ in 0..(MAX_GENERATED_TEXT_BYTES / chunk.len()) {
        buffer.push_generated_text(&chunk).unwrap();
    }
    assert_eq!(buffer.as_str().len(), MAX_GENERATED_TEXT_BYTES);
    let error = buffer.push_generated_text("z").unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(buffer.as_str().len(), MAX_GENERATED_TEXT_BYTES);
}

#[test]
fn generated_audio_buffer_accepts_exact_byte_limit_and_rejects_overflow_without_growth() {
    let mut buffer = GeneratedAudioBuffer::default();
    let exact = vec![0_u8; MAX_GENERATED_AUDIO_BYTES];
    buffer
        .push_generated_audio(&exact, 44_100, 2, PcmSampleFormat::S16Le)
        .unwrap();
    assert_eq!(buffer.pcm().len(), MAX_GENERATED_AUDIO_BYTES);

    let error = buffer
        .push_generated_audio(&[0, 0, 0, 0], 44_100, 2, PcmSampleFormat::S16Le)
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(buffer.pcm().len(), MAX_GENERATED_AUDIO_BYTES);
}

#[test]
fn generated_audio_buffer_enforces_duration_limit_before_append() {
    let mut buffer = GeneratedAudioBuffer::default();
    let exact = vec![0_u8; 600];
    buffer
        .push_generated_audio(&exact, 1, 1, PcmSampleFormat::S16Le)
        .unwrap();
    assert_eq!(
        buffer.duration_millis(),
        Some(MAX_GENERATED_AUDIO_DURATION_MILLIS)
    );

    let error = buffer
        .push_generated_audio(&[0, 0], 1, 1, PcmSampleFormat::S16Le)
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(buffer.pcm().len(), 600);
}

#[test]
fn generated_video_buffer_enforces_byte_frame_and_duration_limits_without_tail_retention() {
    let mut byte_buffer = GeneratedVideoBuffer::default();
    let exact = vec![7_u8; MAX_GENERATED_VIDEO_BYTES];
    byte_buffer.push_generated_frame(&exact, 0).unwrap();
    let error = byte_buffer.push_generated_frame(&[8], 1).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(byte_buffer.frames().len(), 1);
    assert_eq!(
        byte_buffer.frames()[0].encoded_frame.len(),
        MAX_GENERATED_VIDEO_BYTES
    );

    let mut frame_buffer = GeneratedVideoBuffer::default();
    for timestamp in 0..u64::try_from(MAX_GENERATED_VIDEO_FRAMES).unwrap() {
        frame_buffer.push_generated_frame(&[1], timestamp).unwrap();
    }
    let error = frame_buffer
        .push_generated_frame(&[1], u64::try_from(MAX_GENERATED_VIDEO_FRAMES).unwrap())
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(frame_buffer.frames().len(), MAX_GENERATED_VIDEO_FRAMES);

    let mut duration_buffer = GeneratedVideoBuffer::default();
    duration_buffer.push_generated_frame(&[1], 0).unwrap();
    duration_buffer
        .push_generated_frame(&[1], MAX_GENERATED_VIDEO_DURATION_MICROS)
        .unwrap();
    let error = duration_buffer
        .push_generated_frame(&[1], MAX_GENERATED_VIDEO_DURATION_MICROS + 1)
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(duration_buffer.frames().len(), 2);
}

