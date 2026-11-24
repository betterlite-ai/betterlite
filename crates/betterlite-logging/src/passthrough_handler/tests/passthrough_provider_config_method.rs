use super::support::{harness, headers, json, upstream};

#[test]
fn asks_the_provider_for_the_request_model() {
    let harness = harness();
    harness
        .provider
        .transform_ok(json(r#"{"id": "chatcmpl-1"}"#));
    let config = harness
        .handler
        .provider
        .get_provider_config("claude-3-opus");
    assert_eq!(
        harness.provider.config_models(),
        vec!["claude-3-opus".to_owned()]
    );
    let headers = headers();
    let transformed = config
        .transform_response(upstream(&headers), "claude-3-opus", &harness.logging)
        .unwrap();
    assert_eq!(transformed, json(r#"{"id": "chatcmpl-1"}"#));
    let calls = harness.provider.transforms();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "claude-3-opus");
    assert_eq!(calls[0].call_id.as_deref(), Some("call-1"));
}

#[test]
fn forwards_an_empty_model_unchanged() {
    let harness = harness();
    harness.handler.provider.get_provider_config("");
    assert_eq!(harness.provider.config_models(), vec![String::new()]);
}

#[test]
fn asks_once_for_each_call() {
    let harness = harness();
    harness.handler.provider.get_provider_config("claude-3");
    harness
        .handler
        .provider
        .get_provider_config("claude-3-haiku");
    assert_eq!(
        harness.provider.config_models(),
        vec!["claude-3".to_owned(), "claude-3-haiku".to_owned()]
    );
}

#[test]
fn a_config_without_a_scripted_reply_fails_typed() {
    let harness = harness();
    let config = harness.handler.provider.get_provider_config("claude-3");
    let headers = headers();
    let error = config
        .transform_response(upstream(&headers), "claude-3", &harness.logging)
        .expect_err("the fake config has no reply");
    assert_eq!(error.to_string(), "the fake probe failed");
}
