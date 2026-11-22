#![cfg(test)]

use crate::azure_dispatch::call::{Composition, Credentials, Globals, Request};
use crate::azure_dispatch::compose::prepare;
use crate::azure_dispatch::error::AzureProviderError;
use crate::azure_dispatch::fixtures::execution::dispatch;

use super::support::{self, Fixture};

fn identity(base: &str, version: &str, key: &str, token: Option<&str>) -> Credentials {
    Credentials {
        api_base: Some(String::from(base)),
        api_version: Some(String::from(version)),
        api_key: Some(support::key(key)),
        azure_ad_token: token.map(support::key),
        ..support::credentials()
    }
}

fn composition_with(base: &str, key: &str) -> Composition {
    let mut composition = support::composition(&[]);
    composition.globals = Globals {
        api_base: Some(String::from(base)),
        api_key: Some(support::key(key)),
        api_version: None,
        azure_key: None,
        headers: None,
        azure_default_api_version: None,
    };
    composition
}

// Two configured identities never share a base, a version, a key or a header map, and the same identity composes the same request again.
#[test]
fn azure_provider_credential_identity_isolation() {
    call_identities(&support::composition(&[]));
    global_identities();
}

fn call_identities(composition: &Composition) {
    let first = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat(
        "first-model",
        support::object("{}"),
        identity("https://first.test", "2024-01-01", "first-key", None),
    );
    dispatch(composition, Request::Chat(Box::new(call)), &first);
    let second = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat(
        "second-model",
        support::object("{}"),
        identity(
            "https://second.test",
            "2025-05-05",
            "second-key",
            Some("second-token"),
        ),
    );
    dispatch(composition, Request::Chat(Box::new(call)), &second);
    let first_sent = first.sent();
    let second_sent = second.sent();
    assert_eq!(first_sent.len(), 1);
    assert_eq!(second_sent.len(), 1);
    assert_eq!(
        first_sent[0].url,
        "https://first.test/openai/deployments/first-model/chat/completions?api-version=2024-01-01"
    );
    assert_eq!(
        support::header(&first_sent[0], "api-key"),
        Some("first-key")
    );
    assert!(support::header(&first_sent[0], "authorization").is_none());
    assert_eq!(
        second_sent[0].url,
        "https://second.test/openai/deployments/second-model/chat/completions?api-version=2025-05-05"
    );
    assert_eq!(
        support::header(&second_sent[0], "authorization"),
        Some("Bearer second-token")
    );
    assert!(support::header(&second_sent[0], "api-key").is_none());
    assert!(!support::sent_text(&first_sent[0]).contains("second"));
    assert!(!support::sent_text(&second_sent[0]).contains("first-key"));
}

// the module globals are a second identity source, resolved per composition
fn global_identities() {
    let left = composition_with("https://left.test", "left-key");
    let right = composition_with("https://right.test", "right-key");
    let left_answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", support::object("{}"), support::credentials());
    dispatch(&left, Request::Chat(Box::new(call)), &left_answer);
    let right_answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", support::object("{}"), support::credentials());
    dispatch(&right, Request::Chat(Box::new(call)), &right_answer);
    assert_eq!(
        support::header(&right_answer.sent()[0], "api-key"),
        Some("right-key")
    );
    assert_eq!(
        right_answer.sent()[0].url,
        "https://right.test/openai/deployments/gpt-4o/chat/completions?api-version=2025-02-01-preview"
    );
    assert_eq!(
        support::header(&left_answer.sent()[0], "api-key"),
        Some("left-key")
    );
    assert!(!support::sent_text(&left_answer.sent()[0]).contains("right"));
}

// Debug output and every owned failure path keep fixture credentials out of sight.
#[test]
fn azure_provider_secret_safe_failure() {
    debug_redaction();
    failed_dispatch();
    invalid_header();
    globals_redaction();
}

fn debug_redaction() {
    let composition = support::composition(&[]);
    let credentials = Credentials {
        api_base: Some(String::from("https://secret.test")),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("fixture-secret-key")),
        azure_ad_token: Some(support::key("fixture-secret-token")),
        ..support::credentials()
    };
    let call = support::chat("gpt-4o", support::object("{}"), credentials);
    let prepared = prepare(&composition, Request::Chat(Box::new(call))).expect("the chat request");
    let debug = format!("{:?}", prepared.http);
    assert!(!debug.contains("fixture-secret-key"));
    assert!(!debug.contains("fixture-secret-token"));
    assert!(prepared.http.headers["authorization"].is_sensitive());
}

// a failed dispatch reports the status and nothing else
fn failed_dispatch() {
    let credentials = Credentials {
        api_key: Some(support::key("fixture-secret-key")),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::CHAT_REPLY);
    let composition = support::composition(&[]);
    let call = support::chat("gpt-4o", support::object("{}"), credentials);
    let result = dispatch(&composition, Request::Chat(Box::new(call)), &answer);
    assert!(answer.sent().is_empty());
    assert_eq!(support::error_status(&result), Some(500));
    assert!(!support::rendered(&result).contains("fixture-secret-key"));
}

// a header HTTP cannot carry fails without echoing its value
fn invalid_header() {
    let credentials = Credentials {
        api_base: Some(String::from("https://secret.test")),
        api_version: Some(String::from("2024-10-21")),
        extra_headers: Some(support::headers(&[("x-call", "fixture-secret-key\n")])),
        ..support::credentials()
    };
    let composition = support::composition(&[]);
    let call = support::chat("gpt-4o", support::object("{}"), credentials);
    let error = prepare(&composition, Request::Chat(Box::new(call)))
        .err()
        .expect("the invalid header");
    assert!(!error.to_string().contains("fixture-secret-key"));
    assert!(matches!(error, AzureProviderError::Header { .. }));
}

// the module globals redact their secrets in Debug too
fn globals_redaction() {
    let globals = Globals {
        api_key: Some(support::key("fixture-secret-key")),
        azure_key: Some(support::key("fixture-secret-token")),
        api_base: None,
        api_version: None,
        headers: None,
        azure_default_api_version: None,
    };
    let debug = format!("{globals:?}");
    assert!(!debug.contains("fixture-secret-key"));
    assert!(!debug.contains("fixture-secret-token"));
}
