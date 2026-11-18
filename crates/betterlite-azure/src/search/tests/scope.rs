use std::sync::atomic::Ordering;

use betterlite_config::sections::azure_search::TOKEN_ENV;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::search::error::AzureSearchError;

use crate::search::{AzureSearchConfig, ENTRA_SCOPE};

use super::support::{endpoint_token_settings, headers, minter, model_settings, token_settings};

pub(super) fn scope_token_header() {
    api_key_becomes_api_key();
    env_token_authorization();
    minted_token_authorization();
    untrusted_base_is_refused();
    mint_failure_gives_advice();
    missing_minter_is_a_value_error();
    trusted_base_allows_the_env_token();
}

fn api_key_becomes_api_key() {
    let settings = token_settings("env-bearer-token");
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let key = SecretString::from("azure-api-key");
    let existing = headers(&[("Authorization", "Bearer caller"), ("x-existing", "kept")]);
    let merged = config
        .validate_environment(existing, Some(&key), None)
        .unwrap();
    assert_eq!(
        merged.get("api-key").map(String::as_str),
        Some("azure-api-key")
    );
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
    assert_eq!(
        merged.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(merged.get("x-existing").map(String::as_str), Some("kept"));
}

fn env_token_authorization() {
    let settings = token_settings("env-bearer-token");
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let merged = config
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer env-bearer-token")
    );
    assert_eq!(merged.get("api-key"), None);
    let empty = SecretString::from("");
    let fallback = config
        .validate_environment(Headers::new(), Some(&empty), None)
        .unwrap();
    assert_eq!(
        fallback.get("Authorization").map(String::as_str),
        Some("Bearer env-bearer-token")
    );
    assert_eq!(fallback.get("api-key"), None);
}

fn minted_token_authorization() {
    let settings = model_settings();
    let fixture = minter(Ok("minted-bearer-token"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let merged = config
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer minted-bearer-token")
    );
    assert_eq!(
        merged.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

fn untrusted_base_is_refused() {
    let settings = token_settings("env-bearer-token");
    let fixture = minter(Ok("minted-bearer-token"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let from_token = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(from_token, AzureSearchError::Credential(_)));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    let settings = model_settings();
    let fixture = minter(Ok("minted-bearer-token"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let from_minter = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(from_minter, AzureSearchError::Credential(_)));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

fn mint_failure_gives_advice() {
    let settings = model_settings();
    let fixture = minter(Err("no identity source"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let error = config
        .validate_environment(Headers::new(), None, None)
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("no credential available"));
    assert!(message.contains(TOKEN_ENV));
    assert!(message.contains(ENTRA_SCOPE));
    assert!(message.contains("no identity source"));
    assert_eq!(error.status_code(), None);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

fn missing_minter_is_a_value_error() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let error = config
        .validate_environment(Headers::new(), None, None)
        .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, AzureSearchError::NoCredential { .. }));
}

fn trusted_base_allows_the_env_token() {
    let endpoint = "https://project.services.ai.azure.com/api/projects/example";
    let settings = endpoint_token_settings(endpoint, "env-bearer-token");
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let caller_base =
        "https://PROJECT.services.ai.azure.com/api/projects/example/openai/v1/responses";
    let merged = config
        .validate_environment(Headers::new(), None, Some(caller_base))
        .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer env-bearer-token")
    );
    let empty_base = config
        .validate_environment(Headers::new(), None, Some(""))
        .unwrap();
    assert_eq!(
        empty_base.get("Authorization").map(String::as_str),
        Some("Bearer env-bearer-token")
    );
}
