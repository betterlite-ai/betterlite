#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use super::error::PillarError;
use super::headers::{KeyContext, prepare_headers};
use super::settings::{FallbackAction, OnFlaggedAction, PillarSettings};

fn pillar_settings(api_key: Option<&str>) -> PillarSettings {
    PillarSettings {
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        api_base: String::from("https://pillar.test"),
        on_flagged_action: OnFlaggedAction::Monitor,
        async_mode: None,
        persist_session: None,
        include_scanners: true,
        include_evidence: true,
        fallback_on_error: FallbackAction::Allow,
        timeout: std::time::Duration::from_secs(5),
    }
}

fn full_context() -> KeyContext {
    KeyContext {
        key_name: Some(String::from("key-name")),
        key_alias: Some(String::from("key-alias")),
        user_id: Some(String::from("user-1")),
        user_email: Some(String::from("user@example.com")),
        team_id: Some(String::from("team-1")),
        team_alias: Some(String::from("team-name")),
        org_id: Some(String::from("org-1")),
    }
}

#[test]
fn the_authorization_and_content_type_headers_are_always_set() {
    let headers = prepare_headers(&pillar_settings(Some("sk-test")), &KeyContext::default())
        .expect("a key is configured");
    assert_eq!(
        headers.get("Authorization"),
        Some(&"Bearer sk-test".to_owned())
    );
    assert_eq!(
        headers.get("Content-Type"),
        Some(&"application/json".to_owned())
    );
}

#[test]
fn the_boolean_configuration_headers_are_lowercase_words() {
    let mut settings = pillar_settings(Some("sk-test"));
    settings.async_mode = Some(true);
    settings.persist_session = Some(false);
    settings.include_scanners = false;
    settings.include_evidence = true;
    let headers = prepare_headers(&settings, &KeyContext::default()).expect("a key is configured");
    assert_eq!(headers.get("plr_async"), Some(&"true".to_owned()));
    assert_eq!(headers.get("plr_persist"), Some(&"false".to_owned()));
    assert_eq!(headers.get("plr_scanners"), Some(&"false".to_owned()));
    assert_eq!(headers.get("plr_evidence"), Some(&"true".to_owned()));
}

#[test]
fn absent_optional_booleans_set_no_header() {
    let headers = prepare_headers(&pillar_settings(Some("sk-test")), &KeyContext::default())
        .expect("a key is configured");
    assert!(!headers.contains_key("plr_async"));
    assert!(!headers.contains_key("plr_persist"));
}

#[test]
fn every_key_context_field_becomes_its_own_header() {
    let headers = prepare_headers(&pillar_settings(Some("sk-test")), &full_context())
        .expect("a key is configured");
    assert_eq!(
        headers.get("X-LiteLLM-Key-Name"),
        Some(&"key-name".to_owned())
    );
    assert_eq!(
        headers.get("X-LiteLLM-Key-Alias"),
        Some(&"key-alias".to_owned())
    );
    assert_eq!(headers.get("X-LiteLLM-User-Id"), Some(&"user-1".to_owned()));
    assert_eq!(
        headers.get("X-LiteLLM-User-Email"),
        Some(&"user@example.com".to_owned())
    );
    assert_eq!(headers.get("X-LiteLLM-Team-Id"), Some(&"team-1".to_owned()));
    assert_eq!(
        headers.get("X-LiteLLM-Team-Name"),
        Some(&"team-name".to_owned())
    );
    assert_eq!(headers.get("X-LiteLLM-Org-Id"), Some(&"org-1".to_owned()));
}

#[test]
fn a_missing_context_field_sets_no_header() {
    let context = KeyContext {
        user_id: Some(String::from("user-1")),
        ..KeyContext::default()
    };
    let headers =
        prepare_headers(&pillar_settings(Some("sk-test")), &context).expect("a key is configured");
    assert!(!headers.contains_key("X-LiteLLM-Key-Name"));
    assert!(!headers.contains_key("X-LiteLLM-Team-Name"));
    assert_eq!(headers.get("X-LiteLLM-User-Id"), Some(&"user-1".to_owned()));
}

#[test]
fn an_empty_context_field_sets_no_header() {
    let context = KeyContext {
        key_name: Some(String::new()),
        user_id: Some(String::from("user-1")),
        ..KeyContext::default()
    };
    let headers =
        prepare_headers(&pillar_settings(Some("sk-test")), &context).expect("a key is configured");
    assert!(!headers.contains_key("X-LiteLLM-Key-Name"));
}

#[rstest]
#[case::missing_key(None)]
#[case::empty_key(Some(""))]
fn a_missing_or_empty_key_is_the_litellm_error(#[case] api_key: Option<&str>) {
    let error = prepare_headers(&pillar_settings(api_key), &KeyContext::default())
        .expect_err("a key is required");
    assert!(matches!(error, PillarError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "Couldn't get Pillar API key, either set the `PILLAR_API_KEY` in the environment or pass it as a parameter to the guardrail in the config file"
    );
}

#[test]
fn the_prepared_headers_are_a_plain_map_with_no_extra_entries() {
    let headers: BTreeMap<String, String> =
        prepare_headers(&pillar_settings(Some("sk-test")), &KeyContext::default())
            .expect("a key is configured");
    assert_eq!(headers.len(), 4);
}
