use super::*;

#[tokio::test]
async fn response_events_rewrite_the_id_provider_and_model() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = model_state("deployment_a");
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
    let created = processor
        .process(CREATED, &mut state, &mut callback, &logging)
        .await
        .expect("the created event is processed")
        .expect("the created event is handed on");
    let expected_id = build_response_id(Some("openai"), Some("deployment_a"), "resp_1");
    let created_json = event_json(&created);
    assert_eq!(
        at(&created_json, &["response", "id"]),
        &Json::Text(expected_id.clone())
    );
    let decoded = decode_response_id(&expected_id);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("resp_1".to_owned()));
    let container = at(&created_json, &["response", "output", "0", "container_id"]);
    let Json::Text(container_id) = container else {
        panic!("the output item keeps a container id");
    };
    let decoded_container = decode_container_id(container_id);
    assert_eq!(
        decoded_container.custom_llm_provider,
        Some(Some("openai".to_owned()))
    );
    assert_eq!(
        decoded_container.model_id,
        Some(Some("deployment_a".to_owned()))
    );
    assert_eq!(decoded_container.response_id, Some("c".to_owned()));
    assert!(state.completed_response.is_none());
    assert!(!state.finished);
}

#[tokio::test]
async fn absent_payload_response_never_rewrites_the_event() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Fixed> = Some(Fixed(decode(COMPLETED)));
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = model_state("deployment_a");
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
        .process(TEXT_DELTA, &mut state, &mut callback, &logging)
        .await
        .expect("the fixed terminal is processed")
        .expect("the fixed terminal is handed on");
    assert_eq!(
        at(&event_json(&event), &["response", "id"]),
        &Json::Text("resp_1".to_owned())
    );
    let stored = state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(
        at(&event_json(stored), &["response", "id"]),
        &Json::Text("resp_1".to_owned())
    );
}

#[tokio::test]
async fn payload_response_on_a_typed_delta_is_rewritten() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = model_state("deployment_a");
    let mut callback = ResponsesStreamHookState::default();
    let logging = logging_object();
    let payload = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","response":{"id":"resp_1","created_at":1,"output":[{"type":"message","id":"msg_1","container_id":"c"}]}}"#;
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
        .process(payload, &mut state, &mut callback, &logging)
        .await
        .expect("the delta is processed")
        .expect("the delta is handed on");
    let json = event_json(&event);
    assert_eq!(
        at(&json, &["response", "id"]),
        &Json::Text(build_response_id(
            Some("openai"),
            Some("deployment_a"),
            "resp_1"
        ))
    );
    assert_eq!(at(&json, &["delta"]), &Json::Text("hi".to_owned()));
    assert_eq!(state.generated_content, "hi");
}

#[tokio::test]
async fn output_item_events_encode_item_containers() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = model_state("deployment_a");
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
    for payload in [ITEM_ADDED, ITEM_DONE, GENERIC_ITEM_ADDED] {
        let event = processor
            .process(payload, &mut state, &mut callback, &logging)
            .await
            .expect("the item event is processed")
            .expect("the item event is handed on");
        let json = event_json(&event);
        let Json::Text(container) = at(&json, &["item", "container_id"]) else {
            panic!("the item keeps a container id");
        };
        let decoded = decode_container_id(container);
        assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
        assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
        assert_eq!(decoded.response_id, Some("c".to_owned()));
    }
}

#[tokio::test]
async fn annotation_and_part_events_encode_containers() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = model_state("deployment_a");
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
    let annotated = processor
        .process(ANNOTATION, &mut state, &mut callback, &logging)
        .await
        .expect("the annotation is processed")
        .expect("the annotation is handed on");
    let annotated_json = event_json(&annotated);
    let Json::Text(container) = at(&annotated_json, &["annotation", "container_id"]) else {
        panic!("the annotation keeps a container id");
    };
    assert_eq!(
        decode_container_id(container).response_id,
        Some("c".to_owned())
    );
    let typed = processor
        .process(PART_TYPED, &mut state, &mut callback, &logging)
        .await
        .expect("the typed part is processed")
        .expect("the typed part is handed on");
    let typed_json = event_json(&typed);
    let Json::Text(container) = at(&typed_json, &["part", "annotations", "0", "container_id"])
    else {
        panic!("the typed part keeps a container id");
    };
    assert_eq!(
        decode_container_id(container).response_id,
        Some("c".to_owned())
    );
    let dict = processor
        .process(PART_DICT, &mut state, &mut callback, &logging)
        .await
        .expect("the dict part is processed")
        .expect("the dict part is handed on");
    let dict_json = event_json(&dict);
    let Json::Text(container) = at(&dict_json, &["part", "annotations", "0", "container_id"])
    else {
        panic!("the dict part keeps a container id");
    };
    assert_eq!(
        decode_container_id(container).response_id,
        Some("c".to_owned())
    );
    let refusal = processor
        .process(PART_REFUSAL, &mut state, &mut callback, &logging)
        .await
        .expect("the refusal part is processed")
        .expect("the refusal part is handed on");
    assert_eq!(
        at(&event_json(&refusal), &["part", "refusal"]),
        &Json::Text("no".to_owned())
    );
}

#[tokio::test]
async fn unrelated_events_pass_through_unchanged() {
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
    let event = processor
        .process(UNRELATED, &mut state, &mut callback, &logging)
        .await
        .expect("the unrelated event is processed")
        .expect("the unrelated event is handed on");
    let parsed: Json = serde_json::from_str(UNRELATED).expect("the fixture parses");
    assert_eq!(event_json(&event), parsed);
}

#[tokio::test]
async fn affinity_wraps_added_and_done_cipher_items() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = affine_state(true, Some("deployment_a"));
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
    for payload in [CIPHER_ADDED, CIPHER_DONE] {
        let event = processor
            .process(payload, &mut state, &mut callback, &logging)
            .await
            .expect("the cipher item is processed")
            .expect("the cipher item is handed on");
        let json = event_json(&event);
        let Json::Text(wrapped) = at(&json, &["item", "encrypted_content"]) else {
            panic!("the item keeps its cipher");
        };
        assert_eq!(wrapped, &wrap_encrypted_content("cipher", "deployment_a"));
        assert_eq!(
            unwrap_encrypted_content(wrapped),
            (Some("deployment_a".to_owned()), "cipher".to_owned())
        );
    }
}

#[tokio::test]
async fn affinity_guards_leave_the_cipher_untouched() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
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
    let cases = [
        (CIPHER_ADDED, affine_state(false, Some("deployment_a"))),
        (CIPHER_ADDED, affine_state(true, None)),
        (CIPHER_ADDED, affine_state(true, Some(""))),
        (CIPHER_EMPTY, affine_state(true, Some("deployment_a"))),
        (CIPHER_NUMBER, affine_state(true, Some("deployment_a"))),
        (
            CIPHER_GENERIC_ITEM,
            affine_state(true, Some("deployment_a")),
        ),
    ];
    for (payload, mut state) in cases {
        let event = processor
            .process(payload, &mut state, &mut callback, &logging)
            .await
            .expect("the guarded event is processed")
            .expect("the guarded event is handed on");
        let parsed: Json = serde_json::from_str(payload).expect("the fixture parses");
        assert_eq!(
            at(&event_json(&event), &["item"]),
            at(&parsed, &["item"]),
            "payload {payload} keeps its cipher"
        );
    }
    let mut delta_state = affine_state(true, Some("deployment_a"));
    let event = processor
        .process(CIPHER_ON_DELTA, &mut delta_state, &mut callback, &logging)
        .await
        .expect("the delta is processed")
        .expect("the delta is handed on");
    assert_eq!(
        at(&event_json(&event), &["item", "encrypted_content"]),
        &Json::Text("cipher".to_owned())
    );
}

#[tokio::test]
async fn affinity_keeps_an_existing_wrapper() {
    let redactor = redactor();
    let ports = Ports::recording();
    let hooks = ports.hooks(&redactor, fixed_clock());
    let transform: Option<Direct> = Some(Direct);
    let tokenization: Option<Tokenization> = None;
    let calculator: Option<Calculator> = None;
    let limits = ApiLimits::default();
    let mut details = IndexMap::new();
    let mut state = affine_state(true, Some("deployment_a"));
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
        .process(CIPHER_ALREADY_WRAPPED, &mut state, &mut callback, &logging)
        .await
        .expect("the wrapped item is processed")
        .expect("the wrapped item is handed on");
    let json = event_json(&event);
    let Json::Text(wrapped) = at(&json, &["item", "encrypted_content"]) else {
        panic!("the item keeps its cipher");
    };
    let existing = "litellm_enc:bW9kZWxfaWQ6ZGVwbG95bWVudF9i;cipher";
    assert_eq!(
        unwrap_encrypted_content(wrapped),
        (Some("deployment_a".to_owned()), existing.to_owned())
    );
}
