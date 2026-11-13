#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use super::CliContext;

#[test]
fn default_matches_litellm_unset_values() {
    let context = CliContext::default();
    assert_eq!(context.base_url, "");
    assert!(!context.base_url_explicit);
    assert!(context.api_key.is_none());
    assert!(!context.api_key_from_token_file);
}

#[test]
fn debug_never_prints_a_held_api_key() {
    let context = CliContext {
        base_url: "https://proxy.example.com".to_owned(),
        base_url_explicit: true,
        api_key: Some(SecretString::from("sk-live-secret-value")),
        api_key_from_token_file: true,
    };
    let debug = format!("{context:?}");
    assert!(!debug.contains("sk-live-secret-value"), "{debug}");
    let cloned = context.clone();
    let held = cloned.api_key.as_ref().expect("the clone holds the key");
    assert_eq!(held.expose_secret(), "sk-live-secret-value");
    assert_eq!(context.base_url, cloned.base_url);
}
