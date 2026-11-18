use super::*;

#[tokio::test]
async fn empty_payload_makes_no_event_and_no_timing_change() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let outcome = processor
        .process("", &mut state, &mut callback, &logging)
        .await;
    assert!(matches!(outcome, Ok(None)));
    assert!(!state.finished);
    assert_eq!(completion_start(&logging), None);
    assert!(ports.success.calls.entries().is_empty());
}

#[tokio::test]
async fn exact_done_marker_finishes_without_an_event() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut spaced_state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let outcome = processor
        .process("[DONE]", &mut state, &mut callback, &logging)
        .await;
    assert!(matches!(outcome, Ok(None)));
    assert!(state.finished);
    assert_eq!(completion_start(&logging), None);
    let spaced = processor
        .process("[DONE] ", &mut spaced_state, &mut callback, &logging)
        .await;
    assert!(matches!(spaced, Ok(None)));
    assert!(!spaced_state.finished);
    assert_eq!(completion_start(&logging), Some(timestamp(CLOCK)));
    assert!(ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn malformed_and_non_object_payloads_are_dropped() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    for payload in [
        "{not json",
        "[]",
        "null",
        "1",
        "\"text\"",
        r#"data:{"type":"response.created"}"#,
    ] {
        let outcome = processor
            .process(payload, &mut state, &mut callback, &logging)
            .await;
        assert!(matches!(outcome, Ok(None)), "payload {payload} must drop");
    }
    assert!(!state.finished);
    assert_eq!(completion_start(&logging), Some(timestamp(CLOCK)));
    await_pending_logging(&mut callback).await;
    assert!(ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn completion_start_time_is_set_once_before_parsing() {
    let redactor = redactor();
    let ports = Ports::recording();
    let clock: Arc<dyn SuccessClock> = Arc::new(TickingClock {
        calls: Mutex::new(0),
        start: timestamp(START),
    });
    let hooks = ports.hooks(&redactor, clock);
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    processor
        .process("{not json", &mut state, &mut callback, &logging)
        .await
        .expect("a malformed payload is skipped");
    assert_eq!(
        completion_start(&logging),
        Some(timestamp(START) + chrono::TimeDelta::seconds(1))
    );
    processor
        .process(TEXT_DELTA, &mut state, &mut callback, &logging)
        .await
        .expect("the delta is handed on");
    assert_eq!(
        completion_start(&logging),
        Some(timestamp(START) + chrono::TimeDelta::seconds(1))
    );
}

#[tokio::test]
async fn missing_provider_config_fails_once_with_the_litellm_message() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = None;
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let first = processor
        .process(CREATED, &mut state, &mut callback, &logging)
        .await;
    let Err(error) = first else {
        panic!("a payload without a provider config must fail");
    };
    assert!(matches!(&error, ProcessEventError::MissingProviderConfig));
    assert_eq!(
        error.to_string(),
        "responses_api_provider_config is required to process live streaming chunks"
    );
    let second = processor
        .process(CREATED, &mut state, &mut callback, &logging)
        .await;
    assert!(second.is_err());
    await_pending_logging(&mut callback).await;
    assert!(state.failure_handled);
    assert_eq!(ports.failure.calls.entries(), vec!["async", "sync"]);
    let deliveries = ports.failure.deliveries();
    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].contains("responses_api_provider_config is required"));
    assert!(ports.success.calls.entries().is_empty());
}

#[tokio::test]
async fn transform_errors_propagate_and_notify_failure_once() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Rejecting> = Some(Rejecting);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let outcome = processor
        .process(TEXT_DELTA, &mut state, &mut callback, &logging)
        .await;
    let Err(error) = outcome else {
        panic!("a rejected transform must surface");
    };
    assert!(matches!(&error, ProcessEventError::Transform(_)));
    assert_eq!(error.to_string(), "the provider transform failed");
    await_pending_logging(&mut callback).await;
    assert_eq!(ports.failure.calls.entries(), vec!["async", "sync"]);
    assert_eq!(ports.failure.deliveries().len(), 1);
}

#[tokio::test]
async fn provider_transform_receives_model_chunk_and_details() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let recording = Recording::new();
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: Some(&recording),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    processor
        .process(TEXT_DELTA, &mut state, &mut callback, &logging)
        .await
        .expect("the delta is processed");
    let seen = recording.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "gpt-4o");
    let parsed: Json = serde_json::from_str(TEXT_DELTA).expect("the fixture parses");
    assert_eq!(seen[0].1, parsed);
    assert_eq!(
        details.get("transform_type"),
        Some(&Json::Text("response.output_text.delta".to_owned()))
    );
    assert_eq!(state.generated_content, "hi");
}

#[tokio::test]
async fn quota_protocol_is_exposed_without_a_per_event_call() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let counter = Arc::new(QuotaCounter {
        calls: Mutex::new(0),
    });
    let port: Arc<dyn ProjectQuotaCallback> = counter.clone();
    port.enforce_frame_quota(None, Some("gpt-4o"), 1, 2).await;
    assert_eq!(counter.calls(), 1);
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed");
    assert_eq!(counter.calls(), 1);
}

#[tokio::test]
async fn payloads_over_the_event_limit_are_dropped_like_malformed_json() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let padding = "x".repeat(limits.max_event_bytes);
    let payload = format!(
        r#"{{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","pad":"{padding}"}}"#
    );
    let outcome = processor
        .process(&payload, &mut state, &mut callback, &logging)
        .await;
    assert!(matches!(outcome, Ok(None)));
    assert_eq!(state.generated_content, "");
    assert!(state.completed_response.is_none());
    assert_eq!(completion_start(&logging), Some(timestamp(CLOCK)));
    assert!(ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn payloads_at_the_event_limit_still_reach_the_provider_transform() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: calculator.as_ref(),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let prefix = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","pad":""#;
    let padding = "x".repeat(limits.max_event_bytes - prefix.len() - 2);
    let payload = format!("{prefix}{padding}\"}}");
    assert_eq!(payload.len(), limits.max_event_bytes);
    processor
        .process(&payload, &mut state, &mut callback, &logging)
        .await
        .expect("the payload at the limit is processed")
        .expect("the payload at the limit is handed on");
    assert_eq!(state.generated_content, "hi");
    assert_eq!(completion_start(&logging), Some(timestamp(CLOCK)));
}
