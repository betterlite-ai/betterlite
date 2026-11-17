async fn after_finish(failure: SourceFailure) -> (bool, usize, usize) {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![
        Ok(finish_chunk()),
        Err(source_error(failure, connection_class("cut"), "cut")),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);
    assert!(stream.next().await.unwrap().is_some());
    let ended = stream.next().await.unwrap().is_none();
    let logged = world.logging.logged.lock().unwrap();
    (ended, logged.completed.len(), logged.failures.len())
}

#[tokio::test]
async fn failure_read_after_finish_ends_the_stream() {
    assert_eq!(after_finish(SourceFailure::Read).await, (true, 1, 0));
}

#[tokio::test]
async fn failure_protocol_after_finish_ends_the_stream() {
    assert_eq!(after_finish(SourceFailure::Protocol).await, (true, 1, 0));
}

#[tokio::test]
async fn failure_read_before_finish_raises_with_partial_usage() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![
        Ok(content_chunk("Hi")),
        Err(source_error(
            SourceFailure::Read,
            connection_class("cut"),
            "cut",
        )),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let error = stream.next().await.unwrap_err();
    assert_eq!(fallback_of(&error).generated_content, "Hi");
    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.failures.len(), 1);
    assert_eq!(logged.partial.len(), 1);
    assert_eq!(logged.partial[0].len(), 1);
    assert_eq!(logged.completed.len(), 0);
}

#[tokio::test]
async fn failure_blocking_skips_partial_usage() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![
        Ok(content_chunk("Hi")),
        Err(source_error(
            SourceFailure::Read,
            connection_class("cut"),
            "cut",
        )),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let error = stream.next().await.unwrap_err();
    assert_eq!(fallback_of(&error).original_status.0, 500);
    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.failures.len(), 1);
    assert!(logged.partial.is_empty());
    assert_eq!(logged.completed.len(), 0);
}

#[tokio::test]
async fn failure_blocking_read_after_finish_raises() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![
        Ok(finish_chunk()),
        Err(source_error(
            SourceFailure::Read,
            connection_class("cut"),
            "cut",
        )),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert!(stream.next().await.unwrap().is_some());
    let error = stream.next().await.unwrap_err();
    assert_eq!(fallback_of(&error).original_status.0, 500);
}

#[tokio::test]
async fn failure_rules_rejection_becomes_a_fallback() {
    let world = World::new();
    world.rules.reject_with("no good");
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("He")]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let error = stream.next().await.unwrap_err();
    assert_eq!(world.rules.seen.lock().unwrap()[0], "He");
    let details = fallback_of(&error);
    assert_eq!(details.generated_content, "He");
    assert!(!details.is_pre_first_chunk);
    assert_eq!(details.original_status.0, 500);
    assert!(details.context.message.contains("rule check"));
    let class = world.logging.logged.lock().unwrap().failures[0].0.clone();
    assert!(matches!(class, ErrorClass::ApiResponseValidation(_)));
}

#[tokio::test]
async fn failure_chunk_error_maps_through_dispatch() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![ProviderChunk::Json(Box::new(JsonValue::String(
        "junk".to_owned(),
    )))]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let error = stream.next().await.unwrap_err();
    let details = fallback_of(&error);
    assert_eq!(details.original_status.0, 500);
    assert_eq!(details.context.model.as_deref(), Some(MODEL));
    let class = world.logging.logged.lock().unwrap().failures[0].0.clone();
    assert!(matches!(class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn failure_repetition_becomes_internal_server() {
    let mut config = config(CallMode::Async);
    config.repeated_chunk_limit = 2;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        content_chunk("loop"),
        content_chunk("loop"),
        content_chunk("loop"),
    ]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_some());
    let error = stream.next().await.unwrap_err();
    assert_eq!(fallback_of(&error).original_status.0, 500);
    let class = world.logging.logged.lock().unwrap().failures[0].0.clone();
    assert!(matches!(class, ErrorClass::InternalServer(_)));
}

#[tokio::test]
async fn failure_stop_iteration_mid_stream_synthesizes_finish() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("custom".to_owned());
    config.provider_kind = ProviderKind::Custom;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk(), empty_choice_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    let finish = only_chunk(stream.next().await);
    assert_eq!(finish_of(&finish).as_deref(), Some("stop"));
    assert!(world.logging.logged.lock().unwrap().failures.is_empty());
}
