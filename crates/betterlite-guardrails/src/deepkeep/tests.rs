#![cfg(test)]

use std::collections::BTreeMap;

use super::{Action, DeepKeepGuardrail, InputType, Inputs, RequestContext, Response};
use betterlite_config::sections::deepkeep_firewall::{Configured, UnreachableFallback};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;

fn guardrail() -> DeepKeepGuardrail {
    #[expect(
        clippy::disallowed_methods,
        reason = "unit tests need a local client for pure request construction"
    )]
    let client = reqwest::Client::builder().build().expect("client");
    DeepKeepGuardrail {
        settings: Configured {
            guardrail_name: "firewall".to_owned(),
            api_key: SecretString::from("key"),
            api_base: "https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api".to_owned(),
            deepkeep_firewall_id: "fixed".to_owned(),
            unreachable_fallback: UnreachableFallback::FailClosed,
            extra_headers: BTreeMap::new(),
        },
        client,
        timeout: super::DEFAULT_TIMEOUT,
    }
}

fn inputs() -> Inputs {
    Inputs {
        texts: vec!["original".to_owned()],
        images: Some(vec![GuardrailValue::Text("image".to_owned())]),
        tools: Some(vec![object(&[("name", text("tool"))])]),
        tool_calls: Some(vec![object(&[("id", text("call"))])]),
        structured_messages: Some(vec![object(&[("role", text("user"))])]),
        model: Some("model".to_owned()),
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

#[test]
fn deepkeep_guardrail_extracts_metadata_with_token_fallback() {
    let data = BTreeMap::from([
        (
            "metadata".to_owned(),
            object(&[("user_api_key_token", text("token"))]),
        ),
        (
            "litellm_metadata".to_owned(),
            object(&[("user_api_key_alias", text("alias"))]),
        ),
    ]);
    let metadata = DeepKeepGuardrail::extract_user_api_key_metadata(&data);
    assert_eq!(
        metadata["user_api_key_hash"],
        GuardrailValue::Text("token".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_alias"],
        GuardrailValue::Text("alias".to_owned())
    );
}

#[test]
fn deepkeep_guardrail_explicit_hash_wins_over_token() {
    let data = BTreeMap::from([(
        "litellm_metadata".to_owned(),
        object(&[
            ("user_api_key_token", text("token")),
            ("user_api_key_hash", text("hash")),
        ]),
    )]);
    let metadata = DeepKeepGuardrail::extract_user_api_key_metadata(&data);
    assert_eq!(
        metadata["user_api_key_hash"],
        GuardrailValue::Text("hash".to_owned())
    );
}

#[test]
fn deepkeep_guardrail_request_keeps_fixed_firewall_id() {
    let request = guardrail().build_request(
        inputs(),
        RequestContext {
            request_data: BTreeMap::new(),
            input_type: InputType::Request,
            litellm_call_id: Some("call".to_owned()),
            litellm_trace_id: Some("trace".to_owned()),
            litellm_version: Some("version".to_owned()),
            dynamic_params: BTreeMap::from([
                ("firewall_id".to_owned(), text("attacker")),
                ("custom".to_owned(), GuardrailValue::Boolean(true)),
            ]),
        },
    );
    assert_eq!(
        request.provider_params["firewall_id"],
        GuardrailValue::Text("fixed".to_owned())
    );
    assert_eq!(
        request.provider_params["custom"],
        GuardrailValue::Boolean(true)
    );
}

#[test]
fn deepkeep_guardrail_empty_replacements_are_preserved() {
    let returned = Response {
        action: Some(Action::None),
        texts: Some(Vec::new()),
        images: Some(Vec::new()),
        tools: Some(Vec::new()),
        tool_calls: Some(Vec::new()),
        structured_messages: Some(Vec::new()),
        blocked_reason: None,
    };
    let output = DeepKeepGuardrail::build_return_inputs(&inputs(), returned).expect("inputs");
    assert!(output.texts.is_empty());
    assert!(output.images.expect("images").is_empty());
    assert!(output.tools.expect("tools").is_empty());
}

#[test]
fn deepkeep_guardrail_blocked_action_has_default_reason() {
    let returned = Response {
        action: Some(Action::Blocked),
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        blocked_reason: None,
    };
    let error = DeepKeepGuardrail::build_return_inputs(&inputs(), returned).expect_err("blocked");
    assert!(error.to_string().contains("Content violates policy"));
}
