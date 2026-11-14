#[tokio::test]
async fn history_returns_the_runtime_openai_messages() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    plan.state.lock().unwrap().messages = vec![human("hi"), ai("done")];
    let messages = handler.history(&ctx).await.unwrap();
    assert_eq!(
        messages,
        vec![
            json_object(r#"{"content":"hi"}"#),
            json_object(r#"{"content":"done"}"#)
        ]
    );
    let configs = plan.state_configs.lock().unwrap();
    let config = configs.first().unwrap();
    assert_eq!(config.thread_id, "session-1");
    assert_eq!(config.recursion_limit, DEFAULT_RECURSION_LIMIT);
    assert!(config.usage_callback.is_none());
}

#[test]
fn history_missing_runtime_answers_nothing() {
    assert!(MissingRuntime.openai_messages(&[]).is_empty());
    assert!(MissingRuntime.general_subagent().is_empty());
    assert!(MissingRuntime.usage_callback(None).is_none());
}

#[tokio::test]
async fn history_returns_an_empty_list_when_the_state_has_none() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    assert!(handler.history(&ctx).await.unwrap().is_empty());
}

#[tokio::test]
async fn history_fails_with_session_closed_before_start() {
    let (runtime, _plan) = FakeRuntime::new();
    let handler = handler(runtime);
    let ctx = context("/work");
    let error = handler.history(&ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::SessionClosed));
}
