#![cfg(test)]

use http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::completion::error::{CompletionError, anthropic_text_error};
use crate::completion::headers::validate_environment;
use crate::completion::reply::decode_legacy_anthropic_reply;

#[test]
fn legacy_credential_identity_isolation() {
    let alice = SecretString::from("sk-alice-identity");
    let bob = SecretString::from("sk-bob-identity");
    let mut shared = HeaderMap::new();
    shared.insert("x-custom", HeaderValue::from_static("kept"));
    let alice_headers = validate_environment(shared.clone(), Some(&alice)).unwrap();
    let bob_headers = validate_environment(shared.clone(), Some(&bob)).unwrap();
    assert_eq!(alice_headers["x-api-key"], "sk-alice-identity");
    assert_eq!(bob_headers["x-api-key"], "sk-bob-identity");
    assert!(
        bob_headers
            .values()
            .all(|value| value != "sk-alice-identity")
    );
    assert!(!shared.contains_key("x-api-key"));
    let later = validate_environment(shared, None);
    assert!(matches!(later, Err(CompletionError::MissingApiKey)));
}

#[test]
fn legacy_secret_safe_failure() {
    let key = SecretString::from("sk-secret-fixture");
    let headers = validate_environment(HeaderMap::new(), Some(&key)).unwrap();
    assert!(!format!("{headers:?}").contains("sk-secret-fixture"));
    assert!(!format!("{key:?}").contains("sk-secret-fixture"));
    let bad = SecretString::from("sk-secret-fixture\nbreak");
    let error = validate_environment(HeaderMap::new(), Some(&bad)).unwrap_err();
    assert!(!format!("{error} {error:?}").contains("sk-secret"));
    let upstream = anthropic_text_error(401, "invalid x-api-key".to_owned());
    assert!(!format!("{upstream:?}").contains("sk-secret"));
    let reply = decode_legacy_anthropic_reply(401, b"not json").unwrap_err();
    assert!(!format!("{reply} {reply:?}").contains("sk-secret"));
}
