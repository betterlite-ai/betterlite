use super::*;

#[tokio::test]
async fn deltas_accumulate_only_string_text_and_arguments() {
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
        TEXT_DELTA,
        ARGS_DELTA,
        CUSTOM_DELTA,
        MCP_DELTA,
        REFUSAL_DELTA,
        NON_STRING_DELTA,
    ] {
        processor
            .process(payload, &mut state, &mut callback, &logging)
            .await
            .expect("the delta is processed");
    }
    assert_eq!(state.generated_content, "hi");
    assert_eq!(state.generated_tool_arguments, "{}XY");
}

#[tokio::test]
async fn terminal_estimation_receives_the_request_and_joined_generation() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let calculator: Option<Calculator> = Some(Calculator::missing());
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    state
        .request_data
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
    state
        .request_data
        .insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
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
    for payload in [TEXT_DELTA, ARGS_DELTA] {
        processor
            .process(payload, &mut state, &mut callback, &logging)
            .await
            .expect("the delta is processed");
    }
    let event = processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    let calls = tokenization
        .as_ref()
        .expect("the fixture tokenization exists")
        .calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o");
    assert_eq!(calls[0].request_input, Json::Text("hello".to_owned()));
    assert_eq!(calls[0].request, Json::Object(state.request_data.clone()));
    assert_eq!(calls[0].generated, "hi{}");
    let json = event_json(&event);
    assert_eq!(
        number_text(at(&json, &["response", "usage", "input_tokens"])),
        "30"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "output_tokens"])),
        "50"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "total_tokens"])),
        "80"
    );
    assert_eq!(number_text(at(&json, &["response", "usage", "cost"])), "");
}

#[tokio::test]
async fn terminals_with_usage_skip_estimation_and_failures_never_estimate() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let calculator: Option<Calculator> = Some(Calculator::missing());
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
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
    let mut with_usage = stream_state();
    let completed = processor
        .process(COMPLETED_USAGE, &mut with_usage, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert_eq!(
        number_text(at(
            &event_json(&completed),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
    let mut failed = stream_state();
    let failure = processor
        .process(FAILED_USAGE, &mut failed, &mut callback, &logging)
        .await
        .expect("the failure is processed")
        .expect("the failure is handed on");
    assert_eq!(
        number_text(at(
            &event_json(&failure),
            &["response", "usage", "input_tokens"]
        )),
        "7"
    );
    let mut bare = stream_state();
    let bare_failure = processor
        .process(FAILED_BARE, &mut bare, &mut callback, &logging)
        .await
        .expect("the bare failure is processed")
        .expect("the bare failure is handed on");
    assert_eq!(
        at(&event_json(&bare_failure), &["response", "usage"]),
        &Json::Null
    );
    assert!(
        tokenization
            .as_ref()
            .expect("the fixture tokenization exists")
            .calls()
            .is_empty()
    );
    await_pending_logging(&mut callback).await;
}

#[tokio::test]
async fn completed_terminal_is_stored_logged_and_returned_without_finishing() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let calculator: Option<Calculator> = Some(Calculator::charging("0.5"));
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    state
        .request_data
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
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
    let event = processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert!(!state.finished);
    assert!(state.completed_response_logged);
    let stored = state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(event_json(stored), event_json(&event));
    assert_eq!(ports.success.calls.entries(), vec!["async", "sync"]);
    let payloads = ports.success.payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        number_text(at(&payloads[0], &["response", "usage", "cost"])),
        "0.5"
    );
    assert_eq!(
        number_text(at(&payloads[0], &["response", "usage", "total_tokens"])),
        "80"
    );
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        "0.5"
    );
    assert!(ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn incomplete_terminal_is_stored_and_logged() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
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
    let event = processor
        .process(INCOMPLETE, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert!(!state.finished);
    assert!(state.completed_response.is_some());
    assert_eq!(
        number_text(at(
            &event_json(&event),
            &["response", "usage", "total_tokens"]
        )),
        "80"
    );
    assert_eq!(ports.success.calls.entries(), vec!["async", "sync"]);
}

#[tokio::test]
async fn failed_terminal_logs_failure_with_usage_billing() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let calculator: Option<Calculator> = Some(Calculator::charging("0.5"));
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
    let event = processor
        .process(FAILED_USAGE, &mut state, &mut callback, &logging)
        .await
        .expect("the failure is processed")
        .expect("the failure is handed on");
    await_pending_logging(&mut callback).await;
    assert!(!state.finished);
    assert!(state.failure_handled);
    assert!(state.completed_response.is_some());
    assert_eq!(ports.failure.calls.entries(), vec!["async", "sync"]);
    assert!(ports.success.calls.entries().is_empty());
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        "0.5"
    );
    let combined = details
        .get("combined_usage_object")
        .expect("the failed usage is recorded");
    assert!(matches!(combined, Json::Object(_)));
    assert_eq!(
        number_text(details.get("response_cost").expect("the cost is recorded")),
        "0.5"
    );
}

#[tokio::test]
async fn generic_terminal_is_billed_as_an_object() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let calculator: Option<Calculator> = Some(Calculator::charging("0.5"));
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
    let event = processor
        .process(GENERIC_COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the generic terminal is processed")
        .expect("the generic terminal is handed on");
    let json = event_json(&event);
    assert_eq!(
        at(&json, &["response", "id"]),
        &Json::Text(build_response_id(Some("openai"), None, "resp_1"))
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "total_tokens"])),
        "16"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "cost"])),
        "0.5"
    );
    assert_eq!(
        at(&json, &["response", "status"]),
        &Json::Null,
        "the unchecked construction fills the defaulted fields"
    );
    assert!(state.completed_response.is_some());
    assert_eq!(ports.success.calls.entries(), vec!["async", "sync"]);
}

#[tokio::test]
async fn terminal_cost_is_stamped_only_when_the_calculator_answers_positive() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = Some(Tokenization::new(30, 50));
    let limits = ApiLimits::default();
    let zero = Calculator::charging("0");
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: Some(&zero),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let event = processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert_eq!(zero.calls(), 1);
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        ""
    );
    let missing = Calculator::missing();
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: Some(&missing),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let event = processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert_eq!(missing.calls(), 1);
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        ""
    );
    let existing = Calculator::charging("0.5");
    let mut details = IndexMap::new();
    let mut state = stream_state();
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let mut processor = ResponsesEventProcessor {
        transform: transform.as_ref(),
        tokenization: tokenization.as_ref(),
        calculator: Some(&existing),
        hooks: &hooks,
        details: &mut details,
        dispatch: Dispatch::Inline,
        limits: &limits,
    };
    let event = processor
        .process(COMPLETED_COST, &mut state, &mut callback, &logging)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert_eq!(existing.calls(), 0);
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        "0.25"
    );
}

#[tokio::test]
async fn terminal_hook_failure_raises_after_failure_delivery() {
    let redactor = redactor();
    let ports = Ports::failing_cache();
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
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await;
    let Err(error) = outcome else {
        panic!("a failing terminal hook must surface");
    };
    assert!(matches!(&error, ProcessEventError::Hook(_)));
    assert_eq!(error.to_string(), "cache write failed");
    await_pending_logging(&mut callback).await;
    assert!(state.completed_response.is_some());
    assert!(state.completed_response_logged);
    assert!(state.failure_handled);
    assert_eq!(ports.failure.calls.entries(), vec!["async", "sync"]);
    assert!(ports.success.calls.entries().is_empty());
}

#[tokio::test]
async fn second_terminal_does_not_log_twice() {
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
    processor
        .process(COMPLETED, &mut state, &mut callback, &logging)
        .await
        .expect("the first terminal is processed");
    let second = processor
        .process(COMPLETED_USAGE, &mut state, &mut callback, &logging)
        .await
        .expect("the second terminal is processed")
        .expect("the second terminal is handed on");
    assert_eq!(ports.success.calls.entries(), vec!["async", "sync"]);
    let stored = state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(
        number_text(at(
            &event_json(stored),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
    assert_eq!(
        number_text(at(
            &event_json(&second),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
}
