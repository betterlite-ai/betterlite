use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::chat::{FoundryChatOptions, PreparedChat};

use super::support::{
    EnvironmentFixture, Keys, NoSources, SENTINEL_KEY, SENTINEL_SECRET, object, settings, validate,
    validate_with,
};

const FOUNDRY_BASE: &str = "https://resource.services.ai.azure.com";
const PLAIN_BASE: &str = "https://studio.example.test";

pub(super) fn credential_identity_isolation() {
    header_identities();
    token_identities();
    prepared_identities();
}

fn header_identities() {
    let first = SecretString::from("key-first");
    let second = SecretString::from("key-second");
    let credentials = settings(None, None);
    let first_headers = validate(
        Keys {
            argument: Some(&first),
            params: None,
            configured: None,
        },
        &credentials,
        Some(FOUNDRY_BASE),
    );
    let second_headers = validate(
        Keys {
            argument: Some(&second),
            params: None,
            configured: None,
        },
        &credentials,
        Some(FOUNDRY_BASE),
    );
    assert_eq!(
        first_headers.get("api-key").map(String::as_str),
        Some("key-first")
    );
    assert_eq!(
        second_headers.get("api-key").map(String::as_str),
        Some("key-second")
    );
    assert!(
        !first_headers
            .values()
            .any(|value| value.contains("key-second"))
    );
    assert!(
        !second_headers
            .values()
            .any(|value| value.contains("key-first"))
    );
}

fn token_identities() {
    let first = settings(Some("token-first"), None);
    let second = settings(Some("token-second"), None);
    let first_headers = validate(Keys::default(), &first, Some(PLAIN_BASE));
    let second_headers = validate(Keys::default(), &second, Some(PLAIN_BASE));
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer token-first")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer token-second")
    );
    assert_ne!(first_headers, second_headers);
}

fn prepared_identities() {
    let first = prepared_with(&SecretString::from("key-first"));
    let second = prepared_with(&SecretString::from("key-second"));
    assert_eq!(
        first.headers.get("api-key").map(String::as_str),
        Some("key-first")
    );
    assert_eq!(
        second.headers.get("api-key").map(String::as_str),
        Some("key-second")
    );
    assert!(
        !first
            .headers
            .values()
            .any(|value| value.contains("key-second"))
    );
    assert_eq!(
        first.url,
        "https://resource.services.ai.azure.com/models/chat/completions"
    );
}

fn prepared_with(key: &SecretString) -> PreparedChat {
    let credentials = settings(None, None);
    let optional = object("{}");
    let options = FoundryChatOptions {
        model: "mistral-large",
        api_version: Some("2024-05-01-preview"),
        messages: &[],
        optional_params: &optional,
        environment: EnvironmentFixture {
            headers: Headers::new(),
            argument_key: Some(key),
            params_key: None,
            configured_key: None,
            credentials: &credentials,
            sources: &NoSources,
            refresh_enabled: false,
            api_base: Some(FOUNDRY_BASE),
        }
        .request(),
    };
    options.prepare().expect("the fixture prepares")
}

pub(super) fn secret_safe_failure() {
    token_failure_hides_secret();
    debug_hides_keys();
}

fn token_failure_hides_secret() {
    let credentials = settings(None, Some(SENTINEL_SECRET));
    let error = validate_with(
        Headers::new(),
        Keys::default(),
        &credentials,
        Some(PLAIN_BASE),
    )
    .expect_err("no source can mint the configured client-secret token");
    assert!(!error.to_string().contains(SENTINEL_SECRET));
    assert!(!format!("{error:?}").contains(SENTINEL_SECRET));
}

fn debug_hides_keys() {
    let key = SecretString::from(SENTINEL_KEY);
    let credentials = settings(None, None);
    let optional = object("{}");
    let options = FoundryChatOptions {
        model: "mistral-large",
        api_version: None,
        messages: &[],
        optional_params: &optional,
        environment: EnvironmentFixture {
            headers: Headers::new(),
            argument_key: Some(&key),
            params_key: None,
            configured_key: None,
            credentials: &credentials,
            sources: &NoSources,
            refresh_enabled: false,
            api_base: Some(FOUNDRY_BASE),
        }
        .request(),
    };
    let request_debug = format!("{:?}", options.environment);
    assert!(!request_debug.contains(SENTINEL_KEY));
    assert!(request_debug.contains("has_argument_key: true"));
    let options_debug = format!("{options:?}");
    assert!(!options_debug.contains(SENTINEL_KEY));
    assert!(options_debug.contains("mistral-large"), "{options_debug}");
    let prepared = options.prepare().expect("the fixture prepares");
    let prepared_debug = format!("{prepared:?}");
    assert!(!prepared_debug.contains(SENTINEL_KEY));
    assert!(prepared_debug.contains("api-key"), "{prepared_debug}");
    assert_eq!(
        prepared.headers.get("api-key").map(String::as_str),
        Some(SENTINEL_KEY)
    );
}
