#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use serde_json::from_str;
use std::collections::BTreeMap;

use super::{merge_agent_card, normalize_protocol_version, resolve_served_version};

type AgentCard = BTreeMap<String, AgentJson>;

fn card(text: &str) -> AgentCard {
    from_str(text).expect("object card")
}

#[test]
fn agent_card_normalizes_supported_semver_forms() {
    let bare: AgentJson = from_str(r#""0.3""#).unwrap();
    let prerelease: AgentJson = from_str(r#""0.3.0-rc1+build.42""#).unwrap();
    let build: AgentJson = from_str(r#""1.0.1+build.42""#).unwrap();
    let unsupported: AgentJson = from_str(r#""2.0.0""#).unwrap();
    let null: AgentJson = from_str("null").unwrap();
    assert_eq!(normalize_protocol_version(&bare), Some("0.3"));
    assert_eq!(normalize_protocol_version(&prerelease), Some("0.3"));
    assert_eq!(normalize_protocol_version(&build), Some("1.0"));
    assert_eq!(normalize_protocol_version(&unsupported), None);
    assert_eq!(normalize_protocol_version(&null), None);
}

#[test]
fn agent_card_resolves_pinned_or_default_version() {
    let pinned = card(r#"{"protocolVersion":"0.3.4"}"#);
    let invalid = card(r#"{"protocolVersion":"not-a-version"}"#);
    assert_eq!(resolve_served_version(Some(&pinned)), "0.3");
    assert_eq!(resolve_served_version(Some(&invalid)), "1.0");
    assert_eq!(resolve_served_version(None), "1.0");
}

#[test]
fn agent_card_merge_filters_and_fills_proxy_fields() {
    let upstream = card(
        r#"{"url":"https://upstream.example/a2a","protocolVersion":"0.3.0","capabilities":{"streaming":true,"pushNotifications":true},"additionalInterfaces":[{"url":"https://bypass.example"}],"securitySchemes":{"upstream":{"type":"apiKey"}},"unknown":"drop me"}"#,
    );
    let merged = merge_agent_card(
        Some(&upstream),
        "https://proxy.example/a2a/id",
        "https://proxy.example",
        None,
        None,
    );
    assert_eq!(merged["url"].as_str(), Some("https://upstream.example/a2a"));
    assert_eq!(merged["protocolVersion"].as_str(), Some("0.3"));
    assert!(!merged.contains_key("additionalInterfaces"));
    assert!(!merged.contains_key("unknown"));
    assert_eq!(
        merged["securitySchemes"]["LiteLLMKey"]["scheme"].as_str(),
        Some("bearer")
    );
    assert_eq!(merged["version"].as_str(), Some("1.0.0"));
}

#[test]
fn agent_card_merge_uses_overrides_and_does_not_mutate_input() {
    let upstream = card(
        r#"{"name":"Upstream","description":"Original","skills":[{"id":"custom"}],"capabilities":{"streaming":false}}"#,
    );
    let original = upstream.clone();
    let merged = merge_agent_card(
        Some(&upstream),
        "https://proxy.example/a2a/id",
        "https://proxy.example",
        Some("Configured"),
        Some("Configured description"),
    );
    assert_eq!(merged["name"].as_str(), Some("Configured"));
    assert_eq!(
        merged["description"].as_str(),
        Some("Configured description")
    );
    assert_eq!(upstream, original);
}
