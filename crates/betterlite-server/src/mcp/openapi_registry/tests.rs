#![cfg(test)]

use super::{OPENAPI_REGISTRY_JSON, openapi_registry};
use sha2::{Digest as _, Sha256};

#[test]
fn openapi_registry_has_the_pinned_entries_in_order() {
    let registry = openapi_registry(OPENAPI_REGISTRY_JSON);
    let names: Vec<&str> = registry.apis.iter().map(|api| api.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "github",
            "atlassian",
            "figma",
            "gmail",
            "google_sheets",
            "google_drive",
            "google_calendar",
            "google_docs",
            "stripe",
            "hubspot",
            "notion",
            "slack",
            "shopify",
            "snowflake",
        ]
    );
}

#[test]
fn openapi_registry_matches_pinned_bytes_and_preserves_omitted_oauth() {
    let registry = openapi_registry(OPENAPI_REGISTRY_JSON);
    let serialized = serde_json::to_string(&registry).unwrap();
    let resource_sha256 = sha256_hex(OPENAPI_REGISTRY_JSON.as_bytes());
    let serialized_sha256 = sha256_hex(serialized.as_bytes());

    assert_eq!(
        resource_sha256,
        "458d22bbb9015f598cdaa49c9d05798d1f0231bf662478dc9ff6d837b2de2d47"
    );
    assert_eq!(
        serialized_sha256,
        "cd64ee259deb5c404dd507001ab7987d53dcab690d7daed6ae7c8825d20f705b"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    hex
}

#[test]
fn openapi_registry_preserves_oauth_presence_pkce_and_ordered_tools() {
    let registry = openapi_registry(OPENAPI_REGISTRY_JSON);
    let slack = registry
        .apis
        .iter()
        .find(|api| api.name == "slack")
        .unwrap();
    let shopify = registry
        .apis
        .iter()
        .find(|api| api.name == "shopify")
        .unwrap();
    let snowflake = registry
        .apis
        .iter()
        .find(|api| api.name == "snowflake")
        .unwrap();
    let atlassian = registry
        .apis
        .iter()
        .find(|api| api.name == "atlassian")
        .unwrap();

    assert_eq!(
        slack.icon_url,
        "https://raw.githubusercontent.com/simple-icons/simple-icons/develop/icons/slack.svg"
    );
    assert_eq!(
        slack.key_tools.first().map(|tool| tool.name.as_str()),
        Some("chat_post_message")
    );
    assert!(shopify.oauth.is_none());
    assert!(snowflake.oauth.is_none());
    assert!(atlassian.oauth.as_ref().is_some_and(|oauth| oauth.pkce));
}

#[test]
fn openapi_registry_uses_empty_apis_when_resource_loading_fails() {
    let registry = openapi_registry("not valid json");
    assert!(registry.apis.is_empty());
}
