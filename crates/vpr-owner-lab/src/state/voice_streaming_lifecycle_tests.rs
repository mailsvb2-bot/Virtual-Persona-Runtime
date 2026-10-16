#[test]
fn retired_playback_registry_rejects_stale_transport_receipts_across_sessions() {
    let (mut engine, _stats, release_tail) = client_text_streaming_voice_engine();
    let playback = engine.voice_playback_registry();
    let (segment_tx, segment_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        engine.voice_turn_streaming(
            sample_pcm(),
            |_| {},
            |segment| {
                segment_tx.send(segment).unwrap();
                Ok(())
            },
        )
    });
    let first = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    playback
        .acknowledge_voice_delivery_sent(
            first.evidence_turn_sequence,
            first.evidence_output_sequence,
        )
        .unwrap();
    release_tail.store(true, Ordering::SeqCst);
    let second = segment_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(worker.join().unwrap().is_ok());
    // Teardown must remove every outstanding handle; a delayed old-session
    // browser receipt cannot promote a segment belonging to a closed session.
    playback.clear();
    for segment in [first, second] {
        assert!(matches!(
            playback.acknowledge_voice_delivery_sent(
                segment.evidence_turn_sequence,
                segment.evidence_output_sequence,
            ),
            Err(LabError::InvalidState)
        ));
        assert!(matches!(
            playback.acknowledge_voice_playback(
                segment.evidence_turn_sequence,
                segment.evidence_output_sequence,
            ),
            Err(LabError::InvalidState)
        ));
    }
}
