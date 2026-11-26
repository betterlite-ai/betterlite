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
fn core_azure_ai_builds_the_foundry_decisions_url() {
    let config = azure_ai();
    let url = (config.complete_url)("https://res.example", "gpt-4o");
    assert_eq!(url, "https://res.example/providers/microsoft/v1/systemone");
}
