#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::Object;

use crate::purview_dlp_evaluation::error_handling::POLICY_BLOCK_STATUS;
use crate::purview_dlp_evaluation::test_support::guardrail_at;
use crate::purview_dlp_evaluation::user_id_blocking::{
    NO_IDENTITY_ERROR, UNTRUSTED_IDENTITY_ERROR, resolve_user_id_for_blocking,
};
use crate::purview_graph_client::user_id_resolution::KeyIdentity;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn data(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

#[test]
fn a_key_bound_user_id_is_accepted() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: Some("user-1"),
        end_user_id: None,
    };
    assert_eq!(
        resolve_user_id_for_blocking(&guardrail.base, &data("{}"), &identity).unwrap(),
        "user-1"
    );
}

#[test]
fn a_key_bound_user_id_wins_over_a_caller_hint() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: Some("trusted"),
        end_user_id: Some("hint"),
    };
    assert_eq!(
        resolve_user_id_for_blocking(&guardrail.base, &data("{}"), &identity).unwrap(),
        "trusted"
    );
}

#[test]
fn an_end_user_id_only_is_rejected() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: None,
        end_user_id: Some("hint"),
    };
    let error = resolve_user_id_for_blocking(&guardrail.base, &data("{}"), &identity).unwrap_err();
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(
        error.detail.get("error"),
        Some(&text(UNTRUSTED_IDENTITY_ERROR))
    );
}

#[test]
fn a_proxy_injected_metadata_user_id_only_is_rejected() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: None,
        end_user_id: None,
    };
    let error = resolve_user_id_for_blocking(
        &guardrail.base,
        &data(r#"{"metadata":{"user_api_key_user_id":"hint"}}"#),
        &identity,
    )
    .unwrap_err();
    assert_eq!(
        error.detail.get("error"),
        Some(&text(UNTRUSTED_IDENTITY_ERROR))
    );
}

#[test]
fn a_configured_metadata_user_id_only_is_rejected() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: None,
        end_user_id: None,
    };
    let error = resolve_user_id_for_blocking(
        &guardrail.base,
        &data(r#"{"metadata":{"user_id":"hint"}}"#),
        &identity,
    )
    .unwrap_err();
    assert_eq!(
        error.detail.get("error"),
        Some(&text(UNTRUSTED_IDENTITY_ERROR))
    );
}

#[test]
fn no_identity_at_all_is_reported_with_its_own_message() {
    let guardrail = guardrail_at("http://127.0.0.1:1");
    let identity = KeyIdentity {
        user_id: None,
        end_user_id: None,
    };
    let error =
        resolve_user_id_for_blocking(&guardrail.base, &data(r#"{"model":"gpt-4"}"#), &identity)
            .unwrap_err();
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(error.detail.get("error"), Some(&text(NO_IDENTITY_ERROR)));
}
