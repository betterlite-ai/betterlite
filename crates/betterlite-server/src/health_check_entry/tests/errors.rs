const SECRET: &str = "sk-abcdefghijklm";

#[tokio::test]
async fn error_probe_failure_uses_the_stack_trace_separator() {
    let harness = Harness::new();
    harness.backend.fail("boom");
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    assert_eq!(failure.error, "boom\nstack trace: boom\nboom\nboom");
    assert!(matches!(failure.exception, HealthEntryError::Probe { .. }));
    assert!(failure.raw_request.is_none());
}

#[tokio::test]
async fn error_probe_failure_reports_the_raw_request() {
    let harness = Harness::new();
    harness.backend.fail("boom");
    let record = RawRequestRecord {
        raw_request_api_base: Some("https://api.example/v1".to_owned()),
        raw_request_body: Some(Json::Object(IndexMap::from([(
            "model".to_owned(),
            Json::Text("my-model".to_owned()),
        )]))),
        raw_request_headers: Some(Json::Object(IndexMap::from([(
            "authorization".to_owned(),
            Json::Text("REDACTED".to_owned()),
        )]))),
        error: None,
    };
    harness.backend.record_raw_request(record.clone());
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    assert_eq!(failure.raw_request, Some(record));
}

#[tokio::test]
async fn error_resolution_failure_with_a_mode_uses_the_normal_text() {
    let harness = Harness::new();
    harness.resolver.failing();
    let outcome = harness
        .run(params(&[("model", "unknown-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    let message = "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model";
    assert_eq!(
        failure.error,
        format!("{message}\nstack trace: {message}\n{message}")
    );
    assert!(failure.raw_request.is_none());
}

#[tokio::test]
async fn error_missing_mode_failure_uses_the_missing_mode_text() {
    let harness = Harness::new();
    harness.resolver.failing();
    let outcome = harness
        .run(params(&[("model", "unknown-model")]), None)
        .await;
    let failure = failed(outcome);
    assert_eq!(
        failure.error,
        "error:LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model. Missing `mode`. Set the `mode` for the model - https://docs.litellm.ai/docs/proxy/health#embedding-models  \nstacktrace: LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model\nLLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model"
    );
    assert!(failure.raw_request.is_none());
}

#[tokio::test]
async fn error_redacts_the_stack_text() {
    let harness = Harness::new();
    harness
        .backend
        .fail(&format!("the key {SECRET} was rejected"));
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    let stack = stack_of(&failure);
    assert!(!stack.contains(SECRET));
    assert!(stack.contains("REDACTED"));
    assert!(
        failure
            .error
            .starts_with(&format!("the key {SECRET} was rejected\nstack trace: "))
    );
}

#[tokio::test]
async fn error_keeps_the_stack_text_when_redaction_is_disabled() {
    let mut harness = Harness::new();
    harness.entry.redact_secrets = false;
    harness
        .backend
        .fail(&format!("the key {SECRET} was rejected"));
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    assert!(stack_of(&failure).contains(SECRET));
}

#[tokio::test]
async fn error_cuts_the_stack_text_to_one_thousand_characters() {
    let harness = Harness::new();
    let message = "x".repeat(600);
    harness.backend.fail(&message);
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    let stack = stack_of(&failure);
    let uncut = format!("{message}\n{message}\n{message}");
    assert_eq!(stack, uncut.chars().take(1000).collect::<String>());
    assert_eq!(stack.chars().count(), 1000);
}

#[tokio::test]
async fn error_cuts_characters_not_bytes() {
    let harness = Harness::new();
    let message = "\u{e9}".repeat(700);
    harness.backend.fail(&message);
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    let stack = stack_of(&failure);
    assert_eq!(stack.chars().count(), 1000);
    assert_eq!(stack.len(), 2000);
}
