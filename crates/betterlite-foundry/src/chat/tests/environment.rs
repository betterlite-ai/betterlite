use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::chat::environment_error::EnvironmentError;

use super::support::{
    Keys, SENTINEL_KEY, SENTINEL_SECRET, SENTINEL_TOKEN, headers, settings, validate, validate_with,
};

const FOUNDRY_BASE: &str = "https://resource.services.ai.azure.com";
const AZURE_OPENAI_BASE: &str = "https://resource.openai.azure.com";
const PLAIN_BASE: &str = "https://studio.example.test";

pub(super) fn foundry_chat_auth() {
    explicit_key_routes_by_host();
    fallback_keys_and_tokens();
    preset_api_key_and_content_type();
    token_failure_is_reported();
}

fn explicit_key_routes_by_host() {
    let key = SecretString::from(SENTINEL_KEY);
    let credentials = settings(None, None);
    let keys = Keys {
        argument: Some(&key),
        ..Keys::default()
    };
    let api_key = headers(&[
        ("Content-Type", "application/json"),
        ("api-key", SENTINEL_KEY),
    ]);
    let bearer = headers(&[
        ("Authorization", &format!("Bearer {SENTINEL_KEY}")),
        ("Content-Type", "application/json"),
    ]);
    assert_eq!(validate(keys, &credentials, Some(FOUNDRY_BASE)), api_key);
    assert_eq!(
        validate(keys, &credentials, Some(AZURE_OPENAI_BASE)),
        api_key
    );
    assert_eq!(validate(keys, &credentials, Some(PLAIN_BASE)), bearer);
    // no base is not an Azure OpenAI v1 host, so the argument key travels as a bearer
    assert_eq!(validate(keys, &credentials, None), bearer);
}

fn fallback_keys_and_tokens() {
    let empty = SecretString::from("");
    let params_key = SecretString::from(SENTINEL_KEY);
    let configured_key = SecretString::from(SENTINEL_KEY);
    let credentials = settings(None, None);
    let api_key = headers(&[
        ("Content-Type", "application/json"),
        ("api-key", SENTINEL_KEY),
    ]);
    // an empty argument key falls through to the request key, then to the configured key
    let keys = Keys {
        argument: Some(&empty),
        params: Some(&params_key),
        ..Keys::default()
    };
    assert_eq!(validate(keys, &credentials, Some(PLAIN_BASE)), api_key);
    let keys = Keys {
        argument: Some(&empty),
        params: Some(&empty),
        configured: Some(&configured_key),
    };
    assert_eq!(validate(keys, &credentials, Some(PLAIN_BASE)), api_key);
    let token_credentials = settings(Some(SENTINEL_TOKEN), None);
    let bearer = headers(&[
        ("Authorization", &format!("Bearer {SENTINEL_TOKEN}")),
        ("Content-Type", "application/json"),
    ]);
    assert_eq!(
        validate(Keys::default(), &token_credentials, Some(PLAIN_BASE)),
        bearer
    );
    // an empty AD token adds no Authorization header
    let empty_token = settings(Some(""), None);
    assert_eq!(
        validate(Keys::default(), &empty_token, Some(PLAIN_BASE)),
        headers(&[("Content-Type", "application/json")])
    );
}

fn preset_api_key_and_content_type() {
    let credentials = settings(Some(SENTINEL_TOKEN), None);
    let initial = headers(&[("api-key", "preset-key"), ("Content-Type", "text/plain")]);
    let validated = validate_with(initial, Keys::default(), &credentials, Some(PLAIN_BASE))
        .expect("a preset api-key skips token resolution");
    assert_eq!(
        validated,
        headers(&[
            ("api-key", "preset-key"),
            ("Content-Type", "application/json")
        ])
    );
}

fn token_failure_is_reported() {
    let credentials = settings(None, Some(SENTINEL_SECRET));
    let error = validate_with(
        Headers::new(),
        Keys::default(),
        &credentials,
        Some(PLAIN_BASE),
    )
    .expect_err("no source can mint the configured client-secret token");
    assert!(matches!(error, EnvironmentError::Token(_)));
    let message = error.to_string();
    assert!(message.contains("client secret"), "{message}");
    assert!(!message.contains(SENTINEL_SECRET), "{message}");
    assert!(!format!("{error:?}").contains(SENTINEL_SECRET));
}
