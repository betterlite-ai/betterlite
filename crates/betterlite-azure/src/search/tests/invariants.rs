use std::sync::atomic::Ordering;

use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::search::AzureSearchConfig;
use crate::search::error::AzureSearchError;

use super::support::{PanicMinter, minter, model_settings, token_settings};

#[test]
fn azure_search_credential_identity_isolation() {
    env_tokens_stay_with_their_settings();
    api_keys_stay_with_their_config();
    minters_stay_with_their_config();
    untrusted_bases_never_mint();
}

fn env_tokens_stay_with_their_settings() {
    let first_settings = token_settings("fixture-identity-a");
    let second_settings = token_settings("fixture-identity-b");
    let first = AzureSearchConfig {
        settings: &first_settings,
        minter: None,
    };
    let second = AzureSearchConfig {
        settings: &second_settings,
        minter: None,
    };
    let first_headers = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    let second_headers = second
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-identity-a")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-identity-b")
    );
    assert_ne!(first_headers, second_headers);
    let repeated = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(repeated, first_headers);
}

fn api_keys_stay_with_their_config() {
    let settings = token_settings("fixture-identity-a");
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let key = SecretString::from("fixture-api-key");
    let keyed = config
        .validate_environment(Headers::new(), Some(&key), None)
        .unwrap();
    assert_eq!(
        keyed.get("api-key").map(String::as_str),
        Some("fixture-api-key")
    );
    assert_eq!(keyed.get("Authorization"), None);
}

fn minters_stay_with_their_config() {
    let first_minter = minter(Ok("fixture-minted-a"));
    let second_minter = minter(Ok("fixture-minted-b"));
    let settings = model_settings();
    let first = AzureSearchConfig {
        settings: &settings,
        minter: Some(&first_minter),
    };
    let second = AzureSearchConfig {
        settings: &settings,
        minter: Some(&second_minter),
    };
    let first_headers = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    let second_headers = second
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-minted-a")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-minted-b")
    );
    assert_eq!(first_minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_minter.calls.load(Ordering::SeqCst), 1);
}

fn untrusted_bases_never_mint() {
    let settings = model_settings();
    let panicking = PanicMinter;
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&panicking),
    };
    let refused = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(refused, AzureSearchError::Credential(_)));
    let fixture = minter(Ok("fixture-unused"));
    let guarded = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    drop(
        guarded
            .validate_environment(Headers::new(), None, Some("https://evil.example"))
            .unwrap_err(),
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn azure_search_secret_safe_failure() {
    settings_and_config_debug_redact();
    credential_failures_do_not_echo_secrets();
    mint_failures_do_not_echo_secrets();
}

fn settings_and_config_debug_redact() {
    let settings = token_settings("fixture-env-secret");
    assert!(!format!("{settings:?}").contains("fixture-env-secret"));
    let fixture = minter(Ok("fixture-minted-secret"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("fixture-env-secret"));
    assert!(!debug.contains("fixture-minted-secret"));
    assert!(debug.contains("has_minter: true"));
}

fn credential_failures_do_not_echo_secrets() {
    let settings = token_settings("fixture-env-secret");
    let fixture = minter(Ok("fixture-minted-secret"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let error = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(error, AzureSearchError::Credential(_)));
    assert!(!error.to_string().contains("fixture-env-secret"));
    assert!(!error.to_string().contains("fixture-minted-secret"));
    assert!(!format!("{error:?}").contains("fixture-env-secret"));
    assert!(!format!("{error:?}").contains("fixture-minted-secret"));
}

fn mint_failures_do_not_echo_secrets() {
    let settings = model_settings();
    let failing = minter(Err("fixture mint failure"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&failing),
    };
    let error = config
        .validate_environment(Headers::new(), None, None)
        .unwrap_err();
    assert!(!error.to_string().contains("fixture-minted-secret"));
    assert!(!format!("{error:?}").contains("fixture-minted-secret"));
    let empty_key = SecretString::from("");
    let succeeding = minter(Ok("fixture-second-mint"));
    let with_empty_key = AzureSearchConfig {
        settings: &settings,
        minter: Some(&succeeding),
    };
    let merged = with_empty_key
        .validate_environment(Headers::new(), Some(&empty_key), None)
        .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer fixture-second-mint")
    );
}
