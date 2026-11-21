use super::support::{Harness, Rig, base_env, call, text_input};
use crate::api::responses_compact_call::compact_responses;
use crate::api::responses_compact_http::service::LogContext;

const BAD_KEY: &str = r#"{"error":{"message":"bad key","type":"invalid_api_key","param":null,"code":"invalid_api_key"}}"#;

pub(super) async fn native_config_required() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let Err(unsupported) = compact_responses(
        &rig.deps(),
        &call("anthropic/claude-sonnet-4", text_input("hello")),
        &mut log,
    )
    .await
    else {
        panic!("a provider without a native responses config must fail");
    };
    assert_eq!(
        unsupported.to_string(),
        "COMPACT responses is not supported for anthropic"
    );
    let failure = unsupported.sdk_failure("anthropic", "claude-sonnet-4", &harness.redactor);
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.status, 500);
    assert!(failure.kind.is_none());
    assert!(failure.param.is_none());
    assert!(failure.code.is_none());
    assert!(rig.wire.requests().is_empty());
}

pub(super) async fn missing_provider() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let Err(absent_provider) = compact_responses(
        &rig.deps(),
        &call("none/gpt-4o", text_input("hello")),
        &mut log,
    )
    .await
    else {
        panic!("a missing provider must fail");
    };
    assert_eq!(
        absent_provider.to_string(),
        "custom_llm_provider is required but passed as None"
    );
    let failure = absent_provider.sdk_failure("", "gpt-4o", &harness.redactor);
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.status, 500);
    assert!(rig.wire.requests().is_empty());
}

pub(super) async fn provider_auth_error() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.wire.queue(401, BAD_KEY);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let Err(rejected_key) = compact_responses(
        &rig.deps(),
        &call("openai/gpt-4o", text_input("hello")),
        &mut log,
    )
    .await
    else {
        panic!("the mock 401 must fail the call");
    };
    let failure = rejected_key.sdk_failure("openai", "gpt-4o", &harness.redactor);
    assert_eq!(failure.class, "AuthenticationError");
    assert_eq!(failure.status, 401);
    assert!(failure.kind.is_none());
    assert!(failure.param.is_none());
    assert!(failure.code.is_none());
    assert!(!failure.message.is_empty());
    assert_eq!(rig.wire.requests().len(), 1);
    assert_eq!(
        rig.source.created().len(),
        1,
        "only the provider call creates a client"
    );
}
