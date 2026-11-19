#[test]
fn post_call_context_reads_the_route_call_type() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let (messages, tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "hi")]);
    assert!(tools.is_none());
}

#[test]
fn post_call_context_takes_the_first_route_call_type_over_the_logging_one() {
    let completion = FakeTranslation {
        messages: Some(vec![message("user", "from-completion")]),
    };
    let embedding = FakeTranslation {
        messages: Some(vec![message("user", "from-embedding")]),
    };
    let registry = registry_of(vec![
        (&completion, CallType::Completion),
        (&embedding, CallType::Embedding),
    ]);
    let routes = RouteCatalog {
        source: "test".to_owned(),
        call_types: vec![CallType::Completion, CallType::Embedding],
        routes: vec![RouteEntry {
            route: "/v1/chat/completions".to_owned(),
            call_types: vec![CallType::Completion, CallType::Embedding],
        }],
    };
    let (messages, _tools) = run_context(
        &routed_data("/v1/chat/completions"),
        Some("embedding"),
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "from-completion")]);
}

#[test]
fn post_call_context_falls_back_to_the_logging_call_type() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let (messages, _tools) = run_context(
        &Object::new(),
        Some("completion"),
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "hi")]);
}

#[test]
fn post_call_context_uses_the_logging_call_type_for_an_unknown_route() {
    let completion = FakeTranslation {
        messages: Some(vec![message("user", "from-completion")]),
    };
    let embedding = FakeTranslation {
        messages: Some(vec![message("user", "from-embedding")]),
    };
    let registry = registry_of(vec![
        (&completion, CallType::Completion),
        (&embedding, CallType::Embedding),
    ]);
    let routes = route_catalog();
    let (messages, _tools) = run_context(
        &routed_data("/v1/unknown"),
        Some("embedding"),
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "from-embedding")]);
}

#[test]
fn post_call_context_falls_back_to_the_request_logging_call_type() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let data = Object::from([(
        "litellm_logging_obj".to_owned(),
        object(vec![("call_type", text("completion"))]),
    )]);
    let (messages, _tools) =
        run_context(&data, Some(""), ScopeFlags::default(), &registry, &routes);
    assert_eq!(messages, vec![plain_message("user", "hi")]);
}

#[test]
fn post_call_context_prefers_the_logging_call_type_over_the_request_one() {
    let completion = FakeTranslation {
        messages: Some(vec![message("user", "from-completion")]),
    };
    let embedding = FakeTranslation {
        messages: Some(vec![message("user", "from-embedding")]),
    };
    let registry = registry_of(vec![
        (&completion, CallType::Completion),
        (&embedding, CallType::Embedding),
    ]);
    let routes = route_catalog();
    let data = Object::from([(
        "litellm_logging_obj".to_owned(),
        object(vec![("call_type", text("completion"))]),
    )]);
    let (messages, _tools) = run_context(
        &data,
        Some("embedding"),
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "from-embedding")]);
}

#[test]
fn post_call_context_returns_nothing_for_an_unknown_call_type() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let data = Object::from([(
        "litellm_logging_obj".to_owned(),
        object(vec![("call_type", text("completion"))]),
    )]);
    let (messages, tools) = run_context(
        &data,
        Some("bogus"),
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert!(messages.is_empty());
    assert!(tools.is_none());
}

#[test]
fn post_call_context_returns_nothing_without_a_call_type() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let (messages, tools) = run_context(
        &Object::new(),
        None,
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert!(messages.is_empty());
    assert!(tools.is_none());
}

#[test]
fn post_call_context_returns_nothing_without_a_translation() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Embedding);
    let routes = route_catalog();
    let (messages, tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert!(messages.is_empty());
    assert!(tools.is_none());
}

#[test]
fn post_call_context_returns_nothing_without_structured_messages() {
    let translation = FakeTranslation { messages: None };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let (messages, tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        ScopeFlags::default(),
        &registry,
        &routes,
    );
    assert!(messages.is_empty());
    assert!(tools.is_none());
    let empty_translation = FakeTranslation {
        messages: Some(Vec::new()),
    };
    let empty_registry = registry_for(&empty_translation, CallType::Completion);
    let (empty_messages, _tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        ScopeFlags::default(),
        &empty_registry,
        &routes,
    );
    assert!(empty_messages.is_empty());
}

#[test]
fn post_call_context_skips_system_messages_when_the_scope_asks() {
    let translation = FakeTranslation {
        messages: Some(vec![message("system", "rules"), message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let scope = ScopeFlags {
        skip_system_message: true,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    let (messages, _tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        scope,
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "hi")]);
}

#[test]
fn post_call_context_skips_tool_messages_when_the_scope_asks() {
    let translation = FakeTranslation {
        messages: Some(vec![message("tool", "result"), message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let scope = ScopeFlags {
        skip_system_message: false,
        skip_tool_message: true,
        scan: ScanScope::AllMessages,
    };
    let (messages, _tools) = run_context(
        &routed_data("/v1/chat/completions"),
        None,
        scope,
        &registry,
        &routes,
    );
    assert_eq!(messages, vec![plain_message("user", "hi")]);
}

#[test]
fn post_call_context_keeps_only_tool_results_in_a_tool_scope() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi"), message("tool", "result")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let scope = ScopeFlags {
        skip_system_message: false,
        skip_tool_message: false,
        scan: ScanScope::ToolResultsOnly,
    };
    let mut data = routed_data("/v1/chat/completions");
    data.insert(
        "tools".to_owned(),
        GuardrailValue::Array(vec![object(vec![("type", text("function"))])]),
    );
    let (messages, tools) = run_context(&data, None, scope, &registry, &routes);
    assert_eq!(messages, vec![plain_message("tool", "result")]);
    assert!(tools.is_none());
}

#[test]
fn post_call_context_returns_nothing_when_every_message_is_skipped() {
    let translation = FakeTranslation {
        messages: Some(vec![message("system", "rules")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let mut data = routed_data("/v1/chat/completions");
    data.insert(
        "tools".to_owned(),
        GuardrailValue::Array(vec![object(vec![("type", text("function"))])]),
    );
    let scope = ScopeFlags {
        skip_system_message: true,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    let (messages, tools) = run_context(&data, None, scope, &registry, &routes);
    assert!(messages.is_empty());
    assert!(tools.is_none());
}

#[test]
fn post_call_context_returns_the_request_tools() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let tools = vec![object(vec![("type", text("function"))])];
    let mut data = routed_data("/v1/chat/completions");
    data.insert("tools".to_owned(), GuardrailValue::Array(tools.clone()));
    let (messages, request_tools) =
        run_context(&data, None, ScopeFlags::default(), &registry, &routes);
    assert_eq!(messages, vec![plain_message("user", "hi")]);
    assert_eq!(request_tools, Some(tools));
}

#[test]
fn post_call_context_omits_empty_or_non_array_tools() {
    let translation = FakeTranslation {
        messages: Some(vec![message("user", "hi")]),
    };
    let registry = registry_for(&translation, CallType::Completion);
    let routes = route_catalog();
    let mut empty = routed_data("/v1/chat/completions");
    empty.insert("tools".to_owned(), GuardrailValue::Array(Vec::new()));
    let (_messages, tools) = run_context(&empty, None, ScopeFlags::default(), &registry, &routes);
    assert!(tools.is_none());
    let mut scalar = routed_data("/v1/chat/completions");
    scalar.insert("tools".to_owned(), text("tool"));
    let (_scalar_messages, scalar_tools) =
        run_context(&scalar, None, ScopeFlags::default(), &registry, &routes);
    assert!(scalar_tools.is_none());
}
