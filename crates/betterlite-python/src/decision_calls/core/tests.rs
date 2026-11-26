#![cfg(test)]

use std::sync::PoisonError;

use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use secrecy::SecretString;

use super::super::convert::{CallWire, ConvertError, parse};
use super::super::error::SessionFailure;
use super::{SessionCore, azure_ai};

fn globals() -> GlobalCredentials {
    GlobalCredentials {
        api_key: Some(SecretString::from("sk-global")),
        api_base: Some(String::from("https://global.example")),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}

#[test]
fn core_reads_the_environment_the_session_was_given() {
    let core: SessionCore = SessionCore::build(r#"{"environment": {"A": "B"}}"#).unwrap();
    let resolution = core.resolution(&globals());
    assert_eq!(
        resolution.environment.get("A").map(String::as_str),
        Some("B")
    );
    assert_eq!(
        resolution.globals.api_base.as_deref(),
        Some("https://global.example")
    );
    assert!(resolution.known_model_names.is_empty());
    let tools = core.tools.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(tools.tools.is_empty());
}

#[test]
fn core_policy_trusts_proxy_headers_for_logging() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let env = core.pre_post_env();
    assert!(env.decision.content);
    assert!(env.decision.key_info);
    assert!(env.input_callbacks.is_empty());
    assert!(env.logger_fn.is_none());
    assert!(env.sentry.is_none());
}

#[test]
fn core_sandbox_uses_the_session_environment() {
    let core: SessionCore =
        SessionCore::build(r#"{"environment":{"E2B_API_KEY":"sk-env"}}"#).unwrap();
    let sandbox = core.sandbox();
    assert_eq!(
        sandbox
            .e2b
            .environment
            .get("E2B_API_KEY")
            .map(String::as_str),
        Some("sk-env")
    );
    assert_eq!(
        sandbox
            .opensandbox
            .environment
            .get("E2B_API_KEY")
            .map(String::as_str),
        Some("sk-env")
    );
}

#[test]
fn core_answers_an_unknown_operation_with_the_operation_it_got() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire = parse(r#"{"operation": "chat"}"#).unwrap();
    let failure = core.call(&wire).unwrap_err();
    let SessionFailure::Invalid(ConvertError::Operation { operation }) = failure else {
        panic!("an unknown operation is a conversion failure");
    };
    assert_eq!(operation, "chat");
}

#[test]
fn core_registers_and_resolves_sandbox_tools() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let register: CallWire = parse(
        r#"{"operation": "register_tools", "arguments": {"tools": [
            {"sandbox_tool_name": "e2b-tool", "litellm_params": {
                "sandbox_provider": "e2b", "api_key": "sk-e2b", "api_base": "https://e2b.example"
            }},
            {"sandbox_tool_name": "skip-me", "litellm_params": {}}
        ], "environment": {}}}"#,
    )
    .unwrap();
    assert_eq!(core.call(&register).unwrap(), "null");
    let resolve: CallWire =
        parse(r#"{"operation": "resolve_tool", "arguments": {"name": "e2b-tool"}}"#).unwrap();
    let answer = core.call(&resolve).unwrap();
    assert!(answer.contains(r#""sandbox_provider":"e2b""#));
    assert!(answer.contains(r#""api_key":"sk-e2b""#));
    let missing: CallWire =
        parse(r#"{"operation": "resolve_tool", "arguments": {"name": "other"}}"#).unwrap();
    assert_eq!(core.call(&missing).unwrap(), "null");
    let cleared: CallWire =
        parse(r#"{"operation": "register_tools", "arguments": {"tools": [], "environment": {}}}"#)
            .unwrap();
    assert_eq!(core.call(&cleared).unwrap(), "null");
    assert_eq!(core.call(&resolve).unwrap(), "null");
}

#[test]
fn core_resolves_sandbox_tool_credentials_from_session_environment() {
    let core: SessionCore =
        SessionCore::build(r#"{"environment":{"E2B_API_KEY":"sk-from-env"}}"#).unwrap();
    let register: CallWire = parse(
        r#"{"operation":"register_tools","arguments":{"environment":{"E2B_API_KEY":"sk-from-env"},"tools":[{"sandbox_tool_name":"env-tool","litellm_params":{"sandbox_provider":"e2b","api_key":"os.environ/E2B_API_KEY"}}]}}"#,
    )
    .unwrap();
    assert_eq!(core.call(&register).unwrap(), "null");
    let resolve: CallWire =
        parse(r#"{"operation":"resolve_tool","arguments":{"name":"env-tool"}}"#).unwrap();
    let answer = core.call(&resolve).unwrap();
    assert!(answer.contains(r#""api_key":"sk-from-env""#));
}

#[test]
fn core_azure_ai_builds_the_foundry_decisions_url() {
    let config = azure_ai();
    let url = (config.complete_url)("https://res.example", "gpt-4o");
    assert_eq!(url, "https://res.example/providers/microsoft/v1/systemone");
}

#[test]
fn core_answers_a_sync_decision_with_an_unusable_timeout_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire =
        parse(r#"{"operation": "decisions", "arguments": {"model": "gpt-4o", "timeout": -1.0}}"#)
            .unwrap();
    let failure = core.call(&wire).unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Timeout { .. })
    ));
}

#[tokio::test]
async fn core_answers_an_async_systemone_timeout_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire =
        parse(r#"{"operation": "systemone", "arguments": {"model": "gpt-4o", "timeout": -1.0}}"#)
            .unwrap();
    let failure = core.acall(&wire).await.unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Timeout { .. })
    ));
}

#[tokio::test]
async fn core_answers_an_unusable_sandbox_create_interval_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire =
        parse(r#"{"operation": "create_sandbox", "arguments": {"poll_interval": -1.0}}"#).unwrap();
    let failure = core.acall(&wire).await.unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Timeout { .. })
    ));
}

#[tokio::test]
async fn core_answers_a_run_with_no_container_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire =
        parse(r#"{"operation": "run_code", "arguments": {"code": "print(1)"}}"#).unwrap();
    let failure = core.acall(&wire).await.unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Container)
    ));
}

#[tokio::test]
async fn core_runs_a_valid_handle_and_propagates_the_missing_provider_key() {
    let core: SessionCore = SessionCore::build(r#"{"environment":{}}"#).unwrap();
    let wire: CallWire = parse(
        r#"{"operation":"run_code","arguments":{"provider":"e2b","code":"print(1)","container":{"id":"sbx-1","provider":"e2b","hidden_params":{"envd_access_token":"envd","traffic_access_token":"traffic","api_base":"not a url"}}}}"#,
    )
    .unwrap();
    assert!(matches!(
        core.acall(&wire).await,
        Err(SessionFailure::Raised(_))
    ));
}

#[tokio::test]
async fn core_answers_a_delete_with_no_container_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire = parse(r#"{"operation": "delete_sandbox", "arguments": {}}"#).unwrap();
    let failure = core.acall(&wire).await.unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Container)
    ));
}

#[tokio::test]
async fn core_answers_an_unusable_code_interpreter_interval_as_a_conversion_failure() {
    let core: SessionCore = SessionCore::build("{}").unwrap();
    let wire: CallWire =
        parse(r#"{"operation": "code_interpreter", "arguments": {"poll_interval": -1.0}}"#)
            .unwrap();
    let failure = core.acall(&wire).await.unwrap_err();
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Timeout { .. })
    ));
}
