const MCP_METADATA: &str =
    r#"{"mcp_list_tools":[{"name":"search"}],"mcp_tool_results":[{"id":"r1"}]}"#;
const EMPTY_LIST_TOOLS: &str = r#"{"mcp_list_tools":[],"mcp_tool_results":["r1"]}"#;

fn mcp_config(mode: CallMode) -> ChatStreamConfig {
    let mut config = config(mode);
    config.mcp_metadata = Some(object(MCP_METADATA));
    config
}

fn world_with_hook(action: HookAction) -> (World, Arc<FakeHook>) {
    let hook = Arc::new(FakeHook::new(action));
    let shared = Arc::clone(&hook);
    let mut world = World::new();
    world.hooks.push(shared);
    (world, hook)
}

#[tokio::test]
async fn mcp_list_tools_and_final_metadata_reach_the_chunk() {
    let (world, hook) = world_with_hook(HookAction::Keep);
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(
        &world,
        &mut correlation,
        mcp_config(CallMode::Async),
        source,
    );

    let chunk = only_chunk(stream.next().await);
    assert!(hidden_usage_total(&chunk).is_some());
    let fields = delta_fields(&chunk).unwrap();
    assert!(fields.contains_key("mcp_list_tools"));
    assert!(fields.contains_key("mcp_tool_results"));

    assert_eq!(
        *hook.call_types.lock().unwrap(),
        vec![Some(CallType::Completion)]
    );
    assert_eq!(hook.seen.lock().unwrap().len(), 1);
    assert!(
        hook.seen.lock().unwrap()[0]
            .hidden_params
            .contains_key("usage")
    );
    let recorded = hook.seen.lock().unwrap()[0].clone();
    let hook_fields = delta_fields(&recorded).unwrap();
    assert!(hook_fields.contains_key("mcp_list_tools"));
    assert!(!hook_fields.contains_key("mcp_tool_results"));
}

#[tokio::test]
async fn mcp_hook_replaces_the_final_chunk() {
    let replacement: StreamChunk = from_str(
        r#"{"id":"replacement","choices":[{"index":0,"delta":{"content":"replaced"},"finish_reason":"stop"}]}"#,
    )
    .unwrap();
    let (world, hook) = world_with_hook(HookAction::Replace(Box::new(replacement)));
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(chunk.id, "replacement");
    assert_eq!(content_of(&chunk), "replaced");
    assert_eq!(hook.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mcp_hook_failure_keeps_the_chunk() {
    let (events, subscriber) = captured();
    let _guard = set_default(subscriber);
    let (world, hook) = world_with_hook(HookAction::Fail);
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(finish_of(&chunk).as_deref(), Some("stop"));
    assert_eq!(hook.seen.lock().unwrap().len(), 1);
    assert_logged(
        &captured_logs(&events),
        "Error in post-call streaming deployment hook",
    );
}

#[tokio::test]
async fn mcp_missing_metadata_adds_nothing() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert!(delta_fields(&only_chunk(stream.next().await)).is_none());
}

#[tokio::test]
async fn mcp_empty_metadata_adds_nothing() {
    let mut config = config(CallMode::Async);
    config.mcp_metadata = Some(JsonObject::new());
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(delta_fields(&only_chunk(stream.next().await)).is_none());
}

#[tokio::test]
async fn mcp_list_tools_only_for_the_first_chunk() {
    let mut config = config(CallMode::Blocking);
    config.stream_options = Some(StreamOptions {
        include_usage: false,
    });
    config.mcp_metadata = Some(object(MCP_METADATA));
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    let chunk = only_chunk(stream.next().await);
    let fields = delta_fields(&chunk).unwrap();
    assert!(fields.contains_key("mcp_list_tools"));
    assert!(!fields.contains_key("mcp_tool_results"));
}

#[tokio::test]
async fn mcp_empty_list_tools_is_not_added() {
    let mut config = config(CallMode::Blocking);
    config.stream_options = Some(StreamOptions {
        include_usage: false,
    });
    config.mcp_metadata = Some(object(EMPTY_LIST_TOOLS));
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(delta_fields(&only_chunk(stream.next().await)).is_none());
}

#[tokio::test]
async fn mcp_list_tools_skip_after_a_shaped_first_chunk() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(
        &world,
        &mut correlation,
        mcp_config(CallMode::Async),
        source,
    );

    let first = only_chunk(stream.next().await);
    assert!(delta_fields(&first).is_none());
    let last = only_chunk(stream.next().await);
    let fields = delta_fields(&last).unwrap();
    assert!(fields.contains_key("mcp_list_tools"));
    assert!(fields.contains_key("mcp_tool_results"));
}

#[tokio::test]
async fn mcp_list_tools_skip_after_a_shaped_first_chunk_without_final_metadata() {
    let mut config = config(CallMode::Blocking);
    config.stream_options = Some(StreamOptions {
        include_usage: false,
    });
    config.mcp_metadata = Some(object(MCP_METADATA));
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    assert!(delta_fields(&only_chunk(stream.next().await)).is_none());
    assert!(delta_fields(&only_chunk(stream.next().await)).is_none());
}

#[rstest::rstest]
#[case::number("5", true)]
#[case::zero("0", false)]
#[case::text("\"abc\"", true)]
#[case::empty_text("\"\"", false)]
#[case::object("{\"a\":1}", true)]
#[case::empty_object("{}", false)]
#[case::null("null", false)]
#[tokio::test]
async fn mcp_list_tools_follows_python_truthiness(#[case] value: &str, #[case] expected: bool) {
    let mut config = config(CallMode::Blocking);
    config.stream_options = Some(StreamOptions {
        include_usage: false,
    });
    config.mcp_metadata = Some(object(&format!(r#"{{"mcp_list_tools":{value}}}"#)));
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config, source);

    let chunk = only_chunk(stream.next().await);
    assert_eq!(
        delta_fields(&chunk).is_some(),
        expected,
        "mcp_list_tools {value}"
    );
}

#[tokio::test]
async fn mcp_hook_sees_only_the_final_chunk() {
    let (world, hook) = world_with_hook(HookAction::Keep);
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    assert_eq!(content_of(&only_chunk(stream.next().await)), "Hi");
    assert!(hook.seen.lock().unwrap().is_empty());
    let last = only_chunk(stream.next().await);
    assert_eq!(finish_of(&last).as_deref(), Some("stop"));
    let seen = hook.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(finish_of(&seen[0]).as_deref(), Some("stop"));
}

#[tokio::test]
async fn mcp_hook_not_called_in_blocking_mode() {
    let (world, hook) = world_with_hook(HookAction::Keep);
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::new(vec![content_chunk("Hi"), finish_chunk()]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Blocking), source);

    let seen = drain(&mut stream).await;
    assert_eq!(seen.len(), 2);
    assert!(hook.seen.lock().unwrap().is_empty());
}
