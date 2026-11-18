#[test]
fn prepare_payload_keeps_the_messages_and_omits_absent_fields() {
    let dynamic = GraySwanDynamicParams::default();
    let messages = vec![plain_message("user", "hi")];
    let payload = prepare_payload(
        &settings(),
        request_with(messages.clone(), &dynamic, &Object::new()),
    );
    assert_eq!(payload.messages, messages);
    assert!(payload.tools.is_none());
    assert!(payload.categories.is_none());
    assert!(payload.policy_id.is_none());
    assert!(payload.reasoning_mode.is_none());
    assert!(payload.metadata.is_none());
    assert!(payload.litellm_metadata.is_none());
}

#[test]
fn prepare_payload_prefers_dynamic_categories_and_falls_back_to_configured() {
    let mut settings = settings();
    settings.categories = BTreeMap::from([("configured".to_owned(), "monitor".to_owned())]);
    let empty = Object::new();
    let dynamic_categories = GraySwanDynamicParams {
        categories: Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())])),
        policy_id: None,
        reasoning_mode: None,
        metadata: None,
    };
    assert_eq!(
        prepare_payload(
            &settings,
            request_with(Vec::new(), &dynamic_categories, &empty)
        )
        .categories,
        Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]))
    );
    let dynamic = GraySwanDynamicParams::default();
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic, &empty)).categories,
        Some(BTreeMap::from([(
            "configured".to_owned(),
            "monitor".to_owned()
        )]))
    );
    let dynamic_empty = GraySwanDynamicParams {
        categories: Some(BTreeMap::new()),
        policy_id: None,
        reasoning_mode: None,
        metadata: None,
    };
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic_empty, &empty)).categories,
        Some(BTreeMap::from([(
            "configured".to_owned(),
            "monitor".to_owned()
        )]))
    );
    assert!(
        prepare_payload(&settings(), request_with(Vec::new(), &dynamic, &empty))
            .categories
            .is_none()
    );
}

#[test]
fn prepare_payload_prefers_the_dynamic_policy_and_falls_back_to_configured() {
    let mut settings = settings();
    settings.policy_id = "configured".to_owned();
    let empty = Object::new();
    let dynamic_policy = GraySwanDynamicParams {
        categories: None,
        policy_id: Some("dynamic".to_owned()),
        reasoning_mode: None,
        metadata: None,
    };
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic_policy, &empty))
            .policy_id
            .as_deref(),
        Some("dynamic")
    );
    let dynamic = GraySwanDynamicParams::default();
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic, &empty))
            .policy_id
            .as_deref(),
        Some("configured")
    );
    let dynamic_empty = GraySwanDynamicParams {
        categories: None,
        policy_id: Some(String::new()),
        reasoning_mode: None,
        metadata: None,
    };
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic_empty, &empty))
            .policy_id
            .as_deref(),
        Some("configured")
    );
    assert!(
        prepare_payload(&settings(), request_with(Vec::new(), &dynamic, &empty))
            .policy_id
            .is_none()
    );
}

#[test]
fn prepare_payload_prefers_the_dynamic_reasoning_mode_and_falls_back_to_configured() {
    let mut settings = settings();
    settings.reasoning_mode = Some(ReasoningMode::Off);
    let empty = Object::new();
    let dynamic_thinking = GraySwanDynamicParams {
        categories: None,
        policy_id: None,
        reasoning_mode: Some(ReasoningMode::Thinking),
        metadata: None,
    };
    assert_eq!(
        prepare_payload(
            &settings,
            request_with(Vec::new(), &dynamic_thinking, &empty)
        )
        .reasoning_mode,
        Some(ReasoningMode::Thinking)
    );
    let dynamic = GraySwanDynamicParams::default();
    assert_eq!(
        prepare_payload(&settings, request_with(Vec::new(), &dynamic, &empty)).reasoning_mode,
        Some(ReasoningMode::Off)
    );
    assert!(
        prepare_payload(&settings(), request_with(Vec::new(), &dynamic, &empty))
            .reasoning_mode
            .is_none()
    );
}

#[test]
fn prepare_payload_keeps_the_dynamic_metadata() {
    let metadata = object(vec![("trace", text("t"))]);
    let dynamic = GraySwanDynamicParams {
        categories: None,
        policy_id: None,
        reasoning_mode: None,
        metadata: Some(metadata.clone()),
    };
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &dynamic, &Object::new()),
    );
    assert_eq!(payload.metadata, Some(metadata));
}

#[test]
fn prepare_payload_keeps_non_empty_tools_and_drops_an_empty_list() {
    let dynamic = GraySwanDynamicParams::default();
    let empty = Object::new();
    let mut request = request_with(Vec::new(), &dynamic, &empty);
    request.tools = Some(vec![text("tool")]);
    assert_eq!(
        prepare_payload(&settings(), request).tools,
        Some(vec![text("tool")])
    );
    let mut request = request_with(Vec::new(), &dynamic, &empty);
    request.tools = Some(Vec::new());
    assert!(prepare_payload(&settings(), request).tools.is_none());
}

#[test]
fn prepare_payload_copies_litellm_metadata() {
    let data = Object::from([(
        "litellm_metadata".to_owned(),
        object(vec![("request-id", text("r1"))]),
    )]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    assert_eq!(
        payload.litellm_metadata,
        Some(object(vec![("request-id", text("r1"))]))
    );
}

#[test]
fn prepare_payload_ignores_a_non_object_litellm_metadata() {
    let data = Object::from([("litellm_metadata".to_owned(), text("nope"))]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    assert!(payload.litellm_metadata.is_none());
}

#[test]
fn prepare_payload_merges_inbound_headers_into_litellm_metadata() {
    let data = Object::from([
        (
            "litellm_metadata".to_owned(),
            object(vec![("headers", object(vec![("x-request-id", text("1"))]))]),
        ),
        (
            "headers".to_owned(),
            object(vec![("shade_scan_id", text("abc"))]),
        ),
    ]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    let Some(GuardrailValue::Object(metadata)) = payload.litellm_metadata else {
        panic!("litellm_metadata");
    };
    let Some(GuardrailValue::Object(headers)) = metadata.get("headers") else {
        panic!("headers");
    };
    assert_eq!(headers.get("shade_scan_id"), Some(&text("abc")));
    assert_eq!(headers.get("x-request-id"), Some(&text("1")));
}

#[test]
fn prepare_payload_lets_the_inbound_header_win_on_a_duplicate() {
    let data = Object::from([
        (
            "litellm_metadata".to_owned(),
            object(vec![(
                "headers",
                object(vec![("shade_scan_id", text("old"))]),
            )]),
        ),
        (
            "headers".to_owned(),
            object(vec![("shade_scan_id", text("new"))]),
        ),
    ]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    let Some(GuardrailValue::Object(metadata)) = payload.litellm_metadata else {
        panic!("litellm_metadata");
    };
    let Some(GuardrailValue::Object(headers)) = metadata.get("headers") else {
        panic!("headers");
    };
    assert_eq!(headers.get("shade_scan_id"), Some(&text("new")));
}

#[test]
fn prepare_payload_replaces_non_object_headers_with_the_inbound_headers() {
    let data = Object::from([
        (
            "litellm_metadata".to_owned(),
            object(vec![("headers", text("bad"))]),
        ),
        (
            "headers".to_owned(),
            object(vec![("shade_scan_id", text("new"))]),
        ),
    ]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    let Some(GuardrailValue::Object(metadata)) = payload.litellm_metadata else {
        panic!("litellm_metadata");
    };
    assert_eq!(
        metadata.get("headers"),
        Some(&object(vec![("shade_scan_id", text("new"))]))
    );
}

#[test]
fn prepare_payload_strips_nul_bytes_from_litellm_metadata() {
    let data = Object::from([(
        "litellm_metadata".to_owned(),
        object(vec![("name", text("a\u{0}b"))]),
    )]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    assert_eq!(
        payload.litellm_metadata,
        Some(object(vec![("name", text("ab"))]))
    );
}

#[test]
fn prepare_payload_marks_values_past_the_depth_limit() {
    let data = Object::from([(
        "litellm_metadata".to_owned(),
        object(vec![("nested", nested(100))]),
    )]);
    let payload = prepare_payload(
        &settings(),
        request_with(Vec::new(), &GraySwanDynamicParams::default(), &data),
    );
    let Some(GuardrailValue::Object(metadata)) = payload.litellm_metadata else {
        panic!("litellm_metadata");
    };
    let top = metadata.get("nested").expect("nested");
    assert_eq!(descend(top, 100), &text("MaxDepthExceeded"));
}
