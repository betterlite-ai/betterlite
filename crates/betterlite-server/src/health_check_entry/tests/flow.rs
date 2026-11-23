#[tokio::test]
async fn flow_builds_the_logging_object_with_litellm_arguments() {
    let harness = Harness::new();
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.backend.take();
    assert_eq!(call.name(), "chat");
    let state = state_of(&call);
    let locked = state.lock().unwrap();
    assert!(locked.model.is_empty());
    assert_eq!(locked.messages, Some(Vec::new()));
    assert_eq!(locked.stream, Some(false));
    assert_eq!(locked.call_type, "acompletion");
    assert_eq!(locked.start_time, fixed_time());
    assert!(locked.request_logging.log_raw_request_response);
    assert!(!locked.request_logging.raw_request_only);
    assert_uuid4(&locked.litellm_call_id);
    assert_uuid4(&locked.function_id);
    assert_uuid4(&locked.litellm_trace_id);
    assert_ne!(locked.litellm_call_id, locked.function_id);
    assert_ne!(locked.litellm_call_id, locked.litellm_trace_id);
    assert_ne!(locked.function_id, locked.litellm_trace_id);
}

#[tokio::test]
async fn flow_tracking_metadata_reaches_the_provider_call() {
    let harness = Harness::new();
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.backend.take();
    let Json::Object(metadata) = &call.parameters()["litellm_metadata"] else {
        panic!("the call carries the tracking metadata");
    };
    assert_eq!(
        metadata["tags"],
        Json::List(vec![Json::Text("litellm-internal-health-check".to_owned())])
    );
}

#[tokio::test]
async fn flow_defaults_a_missing_mode_to_chat() {
    let harness = Harness::new();
    let outcome = harness.run(params(&[("model", "my-model")]), None).await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "chat");
}

#[tokio::test]
async fn flow_keeps_the_params_model_and_sets_cache() {
    let harness = Harness::new();
    harness
        .resolver
        .resolve_to(resolved("resolved-model", "openai"));
    let outcome = harness
        .run(params(&[("model", "openai/original")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.backend.take();
    assert_eq!(
        call.parameters()["model"],
        Json::Text("openai/original".to_owned())
    );
    let cache = IndexMap::from([("no-cache".to_owned(), Json::Bool(true))]);
    assert_eq!(call.parameters()["cache"], Json::Object(cache));
}

#[tokio::test]
async fn flow_resolved_model_and_provider_reach_the_mode_handler() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("gpt-4o", "azure"));
    let outcome = harness
        .run(params(&[("model", "azure/gpt-4o")]), Some("realtime"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let Recorded::Realtime {
        model, provider, ..
    } = harness.backend.take()
    else {
        panic!("the realtime mode sends a realtime call");
    };
    assert_eq!(model, "gpt-4o");
    assert_eq!(provider, "azure");
}

#[tokio::test]
async fn flow_resolved_model_reaches_the_ocr_document_lookup() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("gpt-4o", "azure"));
    let outcome = harness
        .run(params(&[("model", "azure/gpt-4o")]), Some("ocr"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(
        harness.documents.seen(),
        Some(("gpt-4o".to_owned(), "azure".to_owned()))
    );
}

#[tokio::test]
async fn flow_resolution_receives_the_params_provider_values() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            params(&[
                ("model", "my-model"),
                ("custom_llm_provider", "openai"),
                ("api_base", "https://api.example/v1"),
                ("api_key", "sk-test-value"),
            ]),
            Some("chat"),
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(
        harness.resolver.request(),
        Some(RequestedResolution {
            model: "my-model".to_owned(),
            custom_llm_provider: Some("openai".to_owned()),
            api_base: Some("https://api.example/v1".to_owned()),
            api_key: Some("sk-test-value".to_owned()),
        })
    );
}

#[tokio::test]
async fn flow_prompt_reaches_the_completion_call() {
    let harness = Harness::new();
    let outcome = harness
        .entry
        .check_health(
            params(&[("model", "my-model")]),
            Some("completion".to_owned()),
            Some("ping".to_owned()),
            None,
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(
        harness.backend.take().parameters()["prompt"],
        Json::Text("ping".to_owned())
    );
}

#[tokio::test]
async fn flow_input_reaches_the_embedding_call() {
    let harness = Harness::new();
    let outcome = harness
        .entry
        .check_health(
            params(&[("model", "my-model")]),
            Some("embedding".to_owned()),
            None,
            Some(vec!["one".to_owned(), "two".to_owned()]),
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(
        harness.backend.take().parameters()["input"],
        Json::List(vec![
            Json::Text("one".to_owned()),
            Json::Text("two".to_owned())
        ])
    );
}

#[tokio::test]
async fn flow_ocr_document_failure_fails_the_health_check() {
    let harness = Harness::new();
    harness.documents.missing();
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("ocr"))
        .await;
    let failure = failed(outcome);
    assert!(matches!(failure.exception, HealthEntryError::Probe { .. }));
    assert!(failure.error.starts_with(
        "no ocr health check document is available for resolved-model through openai\nstack trace: "
    ));
}

#[tokio::test]
async fn flow_unsupported_mode_fails_with_the_exact_message() {
    let harness = Harness::new();
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("moderation"))
        .await;
    let failure = failed(outcome);
    let message =
        "Mode moderation not supported. See modes here: https://docs.litellm.ai/docs/proxy/health";
    assert_eq!(failure.error, format!("{message}\nstack trace: {message}"));
    assert!(
        matches!(failure.exception, HealthEntryError::UnsupportedMode { mode } if mode == "moderation")
    );
    assert_eq!(harness.backend.count(), 0);
}
