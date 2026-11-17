#[tokio::test]
async fn next_returns_chunks_in_order() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        content_chunk("He"),
        content_chunk("y"),
        finish_chunk(),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let first = only_chunk(stream.next().await);
    assert_eq!(content_of(&first), "He");
    assert_eq!(first.id, CHAT_ID);
    assert_eq!(first.model, MODEL);
    assert_eq!(first.choices[0].delta.role.as_deref(), Some("assistant"));
    assert!(!first.hidden_params.contains_key("usage"));

    let second = only_chunk(stream.next().await);
    assert_eq!(content_of(&second), "y");
    assert!(second.choices[0].delta.role.is_none());

    let last = only_chunk(stream.next().await);
    assert_eq!(finish_of(&last).as_deref(), Some("stop"));
    assert_eq!(hidden_usage_total(&last), Some(0));
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_skips_empty_bytes_chunk() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![ProviderChunk::Bytes(Vec::new()), content_chunk("Hi")]);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(content_of(&chunk), "Hi");
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn next_skips_none_text_chunk() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        ProviderChunk::Text("None".to_owned()),
        content_chunk("Hi"),
    ]);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(content_of(&chunk), "Hi");
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn next_moves_inline_usage_to_hidden_params() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_usage_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(content_of(&chunk), "Hi");
    assert!(chunk.usage.is_none());
    assert_eq!(hidden_usage_total(&chunk), Some(10));
}

#[tokio::test]
async fn next_keeps_a_second_content_usage_chunk_that_only_carries_content() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), content_usage_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let second = only_chunk(stream.next().await);
    assert_eq!(content_of(&second), "Hi");
    assert!(second.usage.is_none());
    assert!(hidden_usage_total(&second).is_none());
}

#[tokio::test]
async fn next_keeps_a_stripped_usage_chunk_that_a_choice_keeps_alive() {
    let finish_usage = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![provider_json(finish_usage)]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(finish_of(&chunk).as_deref(), Some("stop"));
    assert!(chunk.usage.is_none());
    assert_eq!(hidden_usage_total(&chunk), Some(10));
}

#[tokio::test]
async fn next_keeps_a_stripped_usage_chunk_with_a_nonzero_choice_index() {
    let indexed = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":1,"delta":{"provider_specific_fields":{}},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), provider_json(indexed)]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let chunk = only_chunk(stream.next().await);
    assert_eq!(chunk.choices[0].index, 1);
    assert!(chunk.usage.is_none());
    assert!(hidden_usage_total(&chunk).is_none());
}

#[rstest::rstest]
#[case::empty_reasoning(
    r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"reasoning_content":""},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#
)]
#[case::empty_vendor_fields(
    r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"provider_specific_fields":{}},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},"vendor_note":"","vendor_list":[],"vendor_obj":{}}"#
)]
#[case::empty_provider_fields(
    r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},"provider_specific_fields":{}}"#
)]
#[tokio::test]
async fn next_drops_stripped_usage_chunks_that_carry_nothing(#[case] raw: &str) {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), provider_json(raw)]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let after = only_chunk(stream.next().await);
    assert_eq!(finish_of(&after).as_deref(), Some("stop"));
}

#[tokio::test]
async fn next_keeps_a_stripped_usage_chunk_with_a_null_valued_provider_field() {
    let with_field = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},"provider_specific_fields":{"trace":null}}"#;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        content_chunk("Hi"),
        finish_chunk(),
        provider_json(with_field),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    assert_eq!(
        finish_of(&only_chunk(stream.next().await)).as_deref(),
        Some("stop")
    );
    let kept = only_chunk(stream.next().await);
    assert!(kept.usage.is_none());
    assert_eq!(
        kept.provider_specific_fields,
        Some(object(r#"{"trace":null}"#))
    );
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_drops_a_stripped_usage_chunk_with_only_an_empty_role() {
    let empty_role = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":""},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        content_chunk("Hi"),
        finish_chunk(),
        provider_json(empty_role),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    assert_eq!(
        finish_of(&only_chunk(stream.next().await)).as_deref(),
        Some("stop")
    );
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_keeps_a_stripped_usage_chunk_with_a_delta_extra_field() {
    let vendor_delta = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"vendor_note":"kept"},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![
        content_chunk("Hi"),
        finish_chunk(),
        provider_json(vendor_delta),
    ]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    assert_eq!(
        finish_of(&only_chunk(stream.next().await)).as_deref(),
        Some("stop")
    );
    let kept = only_chunk(stream.next().await);
    assert!(kept.usage.is_none());
    let delta = &kept.choices[0].delta;
    assert_eq!(
        delta.extra.get("vendor_note"),
        Some(&JsonValue::String("kept".to_owned()))
    );
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_records_usage_chunk_after_finish() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk(), usage_chunk()]);
    let reads = Arc::clone(&source.reads);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    assert_eq!(
        finish_of(&only_chunk(stream.next().await)).as_deref(),
        Some("stop")
    );
    assert!(stream.next().await.unwrap().is_none());
    // the usage chunk after the finish reason is read, then the source reports its end
    assert_eq!(reads.load(Ordering::SeqCst), 4);

    let completed = world.logging.logged.lock().unwrap().completed.clone();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].len(), 3);
    assert!(completed[0][2].choices.is_empty());
    assert_eq!(
        completed[0][2]
            .usage
            .as_ref()
            .map(|usage| usage.total_tokens),
        Some(10)
    );
}

#[tokio::test]
async fn next_sends_usage_chunk_when_asked() {
    let world = World::with_build(StreamBuild::UsageOnly(Box::new(usage_log(7, 3))));
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk(), usage_chunk()]);
    let mut stream = start(
        &world,
        &mut correlation,
        usage_config(CallMode::Blocking),
        source,
    );

    let seen = drain(&mut stream).await;
    assert_eq!(seen.len(), 3);
    assert_eq!(finish_of(&seen[1]).as_deref(), Some("stop"));
    assert!(!seen[1].hidden_params.contains_key("usage"));
    assert_eq!(
        seen[2].usage.as_ref().map(|usage| usage.total_tokens),
        Some(10)
    );
}

#[tokio::test]
async fn next_synthesizes_finish_when_source_ends() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    let finish = only_chunk(stream.next().await);
    assert_eq!(finish_of(&finish).as_deref(), Some("stop"));
    assert_eq!(hidden_usage_total(&finish), Some(0));
    assert_eq!(world.logging.logged.lock().unwrap().successes.len(), 1);
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_blocking_logs_each_chunk() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let seen = drain(&mut stream).await;
    assert_eq!(seen.len(), 2);
    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.started, Some(0));
    assert_eq!(logged.chunks.len(), 2);
}

#[tokio::test]
async fn next_async_does_not_log_chunks() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let seen = drain(&mut stream).await;
    assert_eq!(seen.len(), 2);
    assert_eq!(world.logging.logged.lock().unwrap().started, Some(0));
    assert!(world.logging.logged.lock().unwrap().chunks.is_empty());
    assert!(
        world.logging.logged.lock().unwrap().successes[0]
            .0
            .is_none()
    );
}

#[tokio::test]
async fn next_disable_skips_chunk_logging() {
    let mut config = config(CallMode::Blocking);
    config.logging.disable = true;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    let seen = drain(&mut stream).await;
    assert_eq!(seen.len(), 2);
    assert!(world.logging.logged.lock().unwrap().chunks.is_empty());
    assert!(world.logging.logged.lock().unwrap().successes.is_empty());
}

#[tokio::test]
async fn next_effective_model_names_driver_chunks() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("openai".to_owned());
    config.logging_provider = Some("custom".to_owned());
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert_eq!(only_chunk(stream.next().await).model, MODEL);
    assert_eq!(only_chunk(stream.next().await).model, "custom/gpt-4o");
}

#[tokio::test]
async fn next_effective_model_survives_a_skipped_chunk() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("openai".to_owned());
    config.logging_provider = Some("custom".to_owned());
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), empty_choice_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert_eq!(only_chunk(stream.next().await).model, MODEL);
    let finish = only_chunk(stream.next().await);
    assert_eq!(finish_of(&finish).as_deref(), Some("stop"));
    assert_eq!(finish.model, "custom/gpt-4o");
}

#[tokio::test]
async fn next_effective_model_keeps_openai_when_the_logging_provider_matches() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("openai".to_owned());
    config.logging_provider = Some("openai".to_owned());
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert_eq!(only_chunk(stream.next().await).model, MODEL);
    assert_eq!(only_chunk(stream.next().await).model, MODEL);
}

#[tokio::test]
async fn next_effective_model_renders_none_when_the_logging_provider_is_unset() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("openai".to_owned());
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert_eq!(only_chunk(stream.next().await).model, MODEL);
    assert_eq!(only_chunk(stream.next().await).model, "None/gpt-4o");
}

#[tokio::test]
async fn next_stamps_base_hidden_params() {
    let mut config = config(CallMode::Blocking);
    config.logging_provider = Some("custom".to_owned());
    config.hidden_params = object(r#"{"model_id":"model-1","api_base":"https://api.example"}"#);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config, source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(
        chunk.hidden_params.get("model_id"),
        Some(&JsonValue::String("model-1".to_owned()))
    );
    assert_eq!(
        chunk.hidden_params.get("api_base"),
        Some(&JsonValue::String("https://api.example".to_owned()))
    );
    assert_eq!(
        chunk.hidden_params.get("response_cost"),
        Some(&JsonValue::Null)
    );
    assert_eq!(
        chunk.hidden_params.get("custom_llm_provider"),
        Some(&JsonValue::String("custom".to_owned()))
    );
}

#[tokio::test]
async fn next_restores_correlation_context() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert_eq!(world.restore.calls.load(Ordering::SeqCst), 0);
    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());
    assert_eq!(world.restore.calls.load(Ordering::SeqCst), 1);
    drop(stream);
    assert_eq!(correlation.trace_id, "restored");
}

#[tokio::test]
async fn next_stop_iteration_synthesizes_finish() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("custom".to_owned());
    config.provider_kind = ProviderKind::Custom;
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk(), empty_choice_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    let finish = only_chunk(stream.next().await);
    assert_eq!(finish_of(&finish).as_deref(), Some("stop"));
    assert!(stream.next().await.unwrap().is_none());
}

#[tokio::test]
async fn next_rules_see_the_content_so_far() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("He"), content_chunk("y")]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_some());
    let seen = world.rules.seen.lock().unwrap();
    assert_eq!(seen[0], "He");
    assert_eq!(seen[1], "Hey");
}
