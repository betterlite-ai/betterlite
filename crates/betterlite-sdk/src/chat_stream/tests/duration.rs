#[tokio::test]
async fn duration_blocking_fails_without_reading() {
    let mut config = config(CallMode::Blocking);
    config.max_streaming_duration = Some(5.0_f64);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config, source);
    world.clock.set(0, 1006.0_f64);

    let error = stream.next().await.unwrap_err();
    assert_eq!(
        error.cause.to_string(),
        "Stream exceeded max streaming duration of 5.0s (elapsed 6.0s)"
    );
    assert_eq!(error.class.metadata().status_code, 408);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(world.logging.logged.lock().unwrap().failures.is_empty());
}

#[tokio::test]
async fn duration_async_logs_the_failure() {
    let mut config = config(CallMode::Async);
    config.max_streaming_duration = Some(5.0_f64);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config, source);
    world.clock.set(0, 1006.0_f64);

    let error = stream.next().await.unwrap_err();
    assert_eq!(
        error.cause.to_string(),
        "Stream exceeded max streaming duration of 5.0s (elapsed 6.0s)"
    );
    assert_eq!(error.class.metadata().status_code, 408);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(world.logging.logged.lock().unwrap().failures.len(), 1);
    assert_eq!(
        world.logging.logged.lock().unwrap().failures[0].1,
        "Stream exceeded max streaming duration of 5.0s (elapsed 6.0s)"
    );
    assert!(world.logging.logged.lock().unwrap().partial.is_empty());
}

#[tokio::test]
async fn duration_within_limit_reads_a_chunk() {
    let mut config = config(CallMode::Async);
    config.max_streaming_duration = Some(5.0_f64);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);
    world.clock.set(0, 1004.0_f64);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
}

#[tokio::test]
async fn duration_exact_limit_still_reads() {
    let mut config = config(CallMode::Async);
    config.max_streaming_duration = Some(5.0_f64);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);
    world.clock.set(0, 1005.0_f64);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
}

#[test]
fn duration_system_clock_is_live() {
    assert!(SystemClock.now().seconds > 0);
}
