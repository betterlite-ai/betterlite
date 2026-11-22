#![cfg(test)]

use crate::azure_dispatch::call::{Credentials, Globals};
use crate::azure_dispatch::compose::prepare_chat_http;
use crate::azure_dispatch::dispatch::dispatch_chat;
use crate::azure_dispatch::error::AzureProviderError;

use super::support::{self, Fixture};

// Two configured identities never share a base, a version, a key or a header map, and the same
// identity composes the same request again with no cache in between.
#[test]
fn azure_provider_credential_identity_isolation() {
    let composition = support::composition(&[]);
    let first = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &composition,
        support::chat(
            "first-model",
            support::object("{}"),
            Credentials {
                api_base: Some(String::from("https://first.test")),
                api_version: Some(String::from("2024-01-01")),
                api_key: Some(support::key("first-key")),
                ..Credentials::default()
            },
        ),
        &first,
    );
    let second = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &composition,
        support::chat(
            "second-model",
            support::object("{}"),
            Credentials {
                api_base: Some(String::from("https://second.test")),
                api_version: Some(String::from("2025-05-05")),
                api_key: Some(support::key("second-key")),
                azure_ad_token: Some(support::key("second-token")),
                ..Credentials::default()
            },
        ),
        &second,
    );
    let first_sent = first.sent();
    let second_sent = second.sent();
    assert_eq!(first_sent.len(), 1);
    assert_eq!(second_sent.len(), 1);
    assert_eq!(
        first_sent[0]["url"],
        "https://first.test/openai/deployments/first-model/chat/completions?api-version=2024-01-01"
    );
    assert_eq!(first_sent[0]["headers"]["api-key"], "first-key");
    assert!(first_sent[0]["headers"].get("authorization").is_none());
    assert_eq!(
        second_sent[0]["url"],
        "https://second.test/openai/deployments/second-model/chat/completions?api-version=2025-05-05"
    );
    assert_eq!(
        second_sent[0]["headers"]["authorization"],
        "Bearer second-token"
    );
    assert!(second_sent[0]["headers"].get("api-key").is_none());
    assert!(!first_sent[0].to_string().contains("second"));
    assert!(!second_sent[0].to_string().contains("first-key"));

    // the module globals are a second identity source, resolved per composition
    let mut left = support::composition(&[]);
    left.globals = Globals {
        api_base: Some(String::from("https://left.test")),
        api_key: Some(support::key("left-key")),
        ..Globals::default()
    };
    let mut right = support::composition(&[]);
    right.globals = Globals {
        api_base: Some(String::from("https://right.test")),
        api_key: Some(support::key("right-key")),
        ..Globals::default()
    };
    let left_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &left,
        support::chat("gpt-4o", support::object("{}"), Credentials::default()),
        &left_answer,
    );
    let right_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &right,
        support::chat("gpt-4o", support::object("{}"), Credentials::default()),
        &right_answer,
    );
    assert_eq!(right_answer.sent()[0]["headers"]["api-key"], "right-key");
    assert_eq!(
        right_answer.sent()[0]["url"],
        "https://right.test/openai/deployments/gpt-4o/chat/completions?api-version=2025-02-01-preview"
    );
    assert_eq!(left_answer.sent()[0]["headers"]["api-key"], "left-key");
    assert!(!left_answer.sent()[0].to_string().contains("right"));
}

// Debug output and every owned failure path keep fixture credentials out of sight.
#[test]
fn azure_provider_secret_safe_failure() {
    let composition = support::composition(&[]);
    let (prepared, _json_mode) = prepare_chat_http(
        &composition,
        support::chat(
            "gpt-4o",
            support::object("{}"),
            Credentials {
                api_base: Some(String::from("https://secret.test")),
                api_version: Some(String::from("2024-10-21")),
                api_key: Some(support::key("fixture-secret-key")),
                azure_ad_token: Some(support::key("fixture-secret-token")),
                ..Credentials::default()
            },
        ),
    )
    .expect("the chat request");
    let debug = format!("{prepared:?}");
    assert!(!debug.contains("fixture-secret-key"));
    assert!(!debug.contains("fixture-secret-token"));
    assert!(prepared.headers["authorization"].is_sensitive());

    // a failed dispatch reports the status and nothing else
    let answer = Fixture::ok(support::CHAT_REPLY);
    let result = dispatch_chat(
        &composition,
        support::chat(
            "gpt-4o",
            support::object("{}"),
            Credentials {
                api_key: Some(support::key("fixture-secret-key")),
                ..Credentials::default()
            },
        ),
        &answer,
    );
    assert!(answer.sent().is_empty());
    let value = support::value(&result);
    assert_eq!(value["error"]["status"], 500);
    assert!(!value.to_string().contains("fixture-secret-key"));

    // a header HTTP cannot carry fails without echoing its value
    let error = prepare_chat_http(
        &composition,
        support::chat(
            "gpt-4o",
            support::object("{}"),
            Credentials {
                api_base: Some(String::from("https://secret.test")),
                api_version: Some(String::from("2024-10-21")),
                extra_headers: Some(support::headers(&[("x-call", "fixture-secret-key\n")])),
                ..Credentials::default()
            },
        ),
    )
    .err()
    .expect("the invalid header");
    assert!(matches!(error, AzureProviderError::Header { .. }));
    assert!(!error.to_string().contains("fixture-secret-key"));

    // the module globals redact their secrets in Debug too
    let globals = Globals {
        api_key: Some(support::key("fixture-secret-key")),
        azure_key: Some(support::key("fixture-secret-token")),
        ..Globals::default()
    };
    let debug = format!("{globals:?}");
    assert!(!debug.contains("fixture-secret-key"));
    assert!(!debug.contains("fixture-secret-token"));
}
