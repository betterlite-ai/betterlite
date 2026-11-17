const COSTED_RESPONSE_WITH_HIDDEN: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10,"cost":0.01},"hidden_params":{"additional_headers":{"llm_provider-x-litellm-response-cost":0.01}}}"#;

fn complete_build() -> StreamBuild {
    StreamBuild::Complete(decision(COMPLETE_RESPONSE))
}

fn json_number(value: &Json) -> Option<i64> {
    let Json::Number(number) = value else {
        return None;
    };
    number.as_i64()
}

fn total_usage_of(value: &Json) -> Option<i64> {
    let Json::Object(fields) = value else {
        return None;
    };
    let usage = fields.get("usage")?;
    let Json::Object(counts) = usage else {
        return None;
    };
    json_number(counts.get("total_tokens")?)
}

#[tokio::test]
async fn finalize_logs_and_caches_once() {
    let mut world = World::with_build(complete_build());
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());
    assert!(stream.next().await.unwrap().is_none());

    let expected = decision(COMPLETE_RESPONSE);
    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.completed.len(), 1);
    assert_eq!(logged.successes, vec![(Some(expected.clone()), false)]);
    drop(logged);
    assert_eq!(
        *world.cache.as_ref().unwrap().stored.lock().unwrap(),
        vec![expected]
    );
}

#[tokio::test]
async fn finalize_returns_usage_chunk_when_asked() {
    let mut world = World::with_build(complete_build());
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(
        &world,
        &mut correlation,
        usage_config(CallMode::Blocking),
        source,
    );

    assert!(stream.next().await.unwrap().is_some());
    let usage = only_chunk(stream.next().await);
    assert_eq!(
        usage.usage.as_ref().map(|counts| counts.total_tokens),
        Some(10)
    );
    assert_eq!(
        usage.extra.get("service_tier"),
        Some(&JsonValue::String("default".to_owned()))
    );
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.completed.len(), 1);
    assert_eq!(logged.successes.len(), 1);
    drop(logged);
    assert_eq!(
        world.cache.as_ref().unwrap().stored.lock().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn finalize_empty_build_logs_the_chunk_for_blocking() {
    let mut world = World::with_build(StreamBuild::Empty);
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.successes.len(), 1);
    let Some(response) = &logged.successes[0].0 else {
        panic!("blocking finalize logs the empty creator chunk");
    };
    assert!(matches!(response, Json::Object(_)));
    drop(logged);
    assert!(
        world
            .cache
            .as_ref()
            .unwrap()
            .stored
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn finalize_empty_build_logs_none_for_async() {
    let mut world = World::with_build(StreamBuild::Empty);
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.successes, vec![(None, false)]);
    drop(logged);
    assert!(
        world
            .cache
            .as_ref()
            .unwrap()
            .stored
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn finalize_disable_skips_success_logging() {
    let mut config = config(CallMode::Blocking);
    config.logging.disable = true;
    let mut world = World::with_build(complete_build());
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.completed.len(), 1);
    assert!(logged.successes.is_empty());
    drop(logged);
    assert_eq!(
        world.cache.as_ref().unwrap().stored.lock().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn finalize_cache_hit_skips_storage() {
    let mut config = config(CallMode::Async);
    config.provider = Some("cached_response".to_owned());
    let mut world = World::with_build(complete_build());
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(
        logged.successes,
        vec![(Some(decision(COMPLETE_RESPONSE)), true)]
    );
    drop(logged);
    assert!(
        world
            .cache
            .as_ref()
            .unwrap()
            .stored
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn finalize_openrouter_cost_lands_in_hidden_params() {
    let mut config = config(CallMode::Blocking);
    config.provider = Some("openrouter".to_owned());
    let world = World::with_build(StreamBuild::Complete(decision(COSTED_RESPONSE)));
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(
        logged.successes,
        vec![(Some(decision(COSTED_RESPONSE_WITH_HIDDEN)), false)]
    );
}

#[tokio::test]
async fn finalize_usage_only_build_logs_the_fallback() {
    let mut world = World::with_build(StreamBuild::UsageOnly(Box::new(usage_log(7, 3))));
    world.cache = Some(FakeCache::new());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());

    let logged = world.logging.logged.lock().unwrap();
    assert_eq!(logged.successes.len(), 1);
    let Some(response) = &logged.successes[0].0 else {
        panic!("the usage-only fallback is logged");
    };
    assert_eq!(total_usage_of(response), Some(10));
    drop(logged);
    assert_eq!(
        world.cache.as_ref().unwrap().stored.lock().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn finalize_async_synthesized_finish_logs_nothing() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_some());
    assert!(world.logging.logged.lock().unwrap().successes.is_empty());
}

#[tokio::test]
async fn finalize_blocking_synthesized_finish_logs_once() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi")]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_some());
    assert_eq!(world.logging.logged.lock().unwrap().successes.len(), 1);
}

#[tokio::test]
async fn finalize_without_cache_stores_nothing() {
    let world = World::with_build(complete_build());
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(stream.next().await.unwrap().is_some());
    assert!(stream.next().await.unwrap().is_none());
    assert_eq!(world.logging.logged.lock().unwrap().successes.len(), 1);
}
