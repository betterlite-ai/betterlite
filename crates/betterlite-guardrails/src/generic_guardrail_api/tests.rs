use std::collections::BTreeMap;

use super::extract_user_api_key_metadata;
use crate::generic_guardrail_contracts::{
    GenericGuardrailAPISettings, GuardrailOptions, UnreachableFallback,
};

#[test]
fn generic_guardrail_metadata_merges_litellm_metadata_first() {
    let metadata = BTreeMap::from([
        ("user_api_key_hash".to_owned(), Some("old".to_owned())),
        ("user_api_key_team_id".to_owned(), Some("team".to_owned())),
    ]);
    let litellm = BTreeMap::from([("user_api_key_hash".to_owned(), Some("preferred".to_owned()))]);
    let result = extract_user_api_key_metadata(Some(&litellm), Some(&metadata));
    assert_eq!(result.user_api_key_hash.as_deref(), Some("preferred"));
    assert_eq!(result.user_api_key_team_id.as_deref(), Some("team"));
}

#[test]
fn generic_guardrail_metadata_token_alias_overrides_hash() {
    let metadata = BTreeMap::from([
        ("user_api_key_hash".to_owned(), Some("hash".to_owned())),
        ("user_api_key_token".to_owned(), Some("token".to_owned())),
    ]);
    let result = extract_user_api_key_metadata(None, Some(&metadata));
    assert_eq!(result.user_api_key_hash.as_deref(), Some("token"));
}

#[test]
fn generic_guardrail_error_policy_is_explicit() {
    let settings = GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        unreachable_fallback: Some(UnreachableFallback::FailOpen),
        fail_on_error: Some(false),
        ..GuardrailOptions::default()
    }));
    assert!(!settings.fail_on_error);
    assert_eq!(settings.unreachable_fallback, UnreachableFallback::FailOpen);
}

#[test]
fn generic_guardrail_fail_on_error_override_unreachable() {
    let settings = GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        unreachable_fallback: Some(UnreachableFallback::FailClosed),
        fail_on_error: Some(true),
        ..GuardrailOptions::default()
    }));
    assert!(settings.fail_on_error);
    assert_eq!(
        settings.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}
