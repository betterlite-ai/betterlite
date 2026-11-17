#[tokio::test]
async fn close_closes_the_source_once() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let closes = Arc::clone(&source.closes);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    stream.close().await;
    stream.close().await;
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    assert!(stream.next().await.unwrap().is_none());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(world.restore.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn close_swallows_and_logs_a_failure() {
    let (events, subscriber) = captured();
    let _guard = set_default(subscriber);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::failing_close();
    let closes = Arc::clone(&source.closes);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    stream.close().await;
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    assert_logged(&captured_logs(&events), "error closing completion_stream");
}

#[tokio::test]
async fn close_restores_the_correlation_context() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(Vec::new());
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    stream.close().await;
    drop(stream);
    assert_eq!(correlation.trace_id, "restored");
}
