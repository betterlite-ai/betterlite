#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::Map;

use crate::api::container_contracts::config::BaseContainerConfig as _;
use crate::containers::OpenAiContainerConfig;

use super::support::{config_with, globals_with, settings_with};

#[test]
fn openai_containers_credential_identity_isolation() {
    let first = config_with(
        settings_with("https://first.test/v1"),
        globals_with(Some("sk-first"), None),
        None,
    );
    let second = config_with(
        settings_with("https://second.test/v1"),
        globals_with(None, Some("sk-second")),
        Some("sk-second-env"),
    );
    let caller = BTreeMap::new();
    identities_keep_their_keys(&first, &second, &caller);
    headers_stay_with_identity(&first, &second, &caller);
    bases_stay_with_their_identity(&first, &second);
}

fn identities_keep_their_keys(
    first: &OpenAiContainerConfig,
    second: &OpenAiContainerConfig,
    caller: &BTreeMap<String, String>,
) {
    let first_headers = first.validate_environment(caller, None);
    let second_headers = second.validate_environment(caller, None);
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-first")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-second")
    );
    assert_ne!(first_headers, second_headers);
    assert_eq!(first.validate_environment(caller, None), first_headers);
    assert_eq!(second.validate_environment(caller, None), second_headers);
    let explicit = first.validate_environment(caller, Some("sk-explicit"));
    assert_eq!(
        explicit.get("Authorization").map(String::as_str),
        Some("Bearer sk-explicit")
    );
    assert_eq!(second.validate_environment(caller, None), second_headers);
    assert!(caller.is_empty());
}

fn headers_stay_with_identity(
    first: &OpenAiContainerConfig,
    second: &OpenAiContainerConfig,
    caller: &BTreeMap<String, String>,
) {
    let mut scoped = BTreeMap::new();
    scoped.insert("x-scope".to_owned(), "first".to_owned());
    let scoped_headers = first.validate_environment(&scoped, None);
    assert_eq!(
        scoped_headers.get("x-scope").map(String::as_str),
        Some("first")
    );
    assert!(
        !second
            .validate_environment(caller, None)
            .contains_key("x-scope")
    );
}

fn bases_stay_with_their_identity(first: &OpenAiContainerConfig, second: &OpenAiContainerConfig) {
    let params = Map::new();
    assert_eq!(
        first.get_complete_url(None, &params).unwrap(),
        "https://first.test/v1/containers"
    );
    assert_eq!(
        second.get_complete_url(None, &params).unwrap(),
        "https://second.test/v1/containers"
    );
}
