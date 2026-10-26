#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use serde_json::{from_str, json};

use super::{AgentCard, merge_agent_card, normalize_protocol_version, resolve_served_version};

const PROXY_URL: &str = "https://proxy.example/a2a/agent-1";
const PROXY_BASE: &str = "https://proxy.example";

fn card(text: &str) -> AgentCard {
    from_str(text).expect("object card")
}

fn value(text: &str) -> AgentJson {
    from_str(text).expect("json value")
}

fn merge(upstream: &str) -> AgentCard {
    merge_agent_card(Some(&card(upstream)), PROXY_URL, PROXY_BASE, None, None)
}

#[rstest]
#[case::bare_old("\"0.3\"", Some("0.3"))]
#[case::bare_new("\"1.0\"", Some("1.0"))]
#[case::patch("\"0.3.0\"", Some("0.3"))]
#[case::long_patch("\"1.0.12\"", Some("1.0"))]
#[case::prerelease("\"0.3.0-rc1\"", Some("0.3"))]
#[case::dotted_prerelease("\"0.3.0-rc.1-x\"", Some("0.3"))]
#[case::build("\"1.0.1+build.42\"", Some("1.0"))]
#[case::prerelease_and_build("\"0.3.0-rc1+build.42\"", Some("0.3"))]
#[case::trailing_newline("\"1.0\\n\"", Some("1.0"))]
#[case::two_newlines("\"1.0\\n\\n\"", None)]
#[case::unsupported_major("\"2.0.0\"", None)]
#[case::unsupported_minor("\"1.1\"", None)]
#[case::leading_zero_major("\"01.0\"", None)]
#[case::major_only("\"1\"", None)]
#[case::trailing_dot("\"1.0.\"", None)]
#[case::four_parts("\"1.0.0.0\"", None)]
#[case::letter_patch("\"1.0.x\"", None)]
#[case::empty_major("\".0\"", None)]
#[case::empty_minor("\"1.\"", None)]
#[case::letter_minor("\"1.a\"", None)]
#[case::suffix_without_patch("\"1.0-rc1\"", None)]
#[case::empty_prerelease("\"1.0.0-\"", None)]
#[case::empty_build("\"1.0.0+\"", None)]
#[case::empty_identifier("\"1.0.0-rc..1\"", None)]
#[case::bad_prerelease_char("\"1.0.0-rc_1\"", None)]
#[case::bad_build_char("\"1.0.0+b_1\"", None)]
#[case::second_plus("\"1.0.0+a+b\"", None)]
#[case::leading_space("\" 1.0\"", None)]
#[case::number("1.0", None)]
#[case::null("null", None)]
fn agent_card_normalizes_protocol_versions(#[case] raw: &str, #[case] expected: Option<&str>) {
    let version: AgentJson = from_str(raw).unwrap();
    assert_eq!(normalize_protocol_version(&version), expected);
}

#[rstest]
#[case::pinned(Some(r#"{"protocolVersion":"0.3.4"}"#), "0.3")]
#[case::pinned_new(Some(r#"{"protocolVersion":"1.0"}"#), "1.0")]
#[case::invalid(Some(r#"{"protocolVersion":"not-a-version"}"#), "1.0")]
#[case::unsupported(Some(r#"{"protocolVersion":"0.2.5"}"#), "1.0")]
#[case::absent(Some("{}"), "1.0")]
#[case::no_card(None, "1.0")]
fn agent_card_resolves_served_version(#[case] upstream: Option<&str>, #[case] expected: &str) {
    let pinned = upstream.map(card);
    assert_eq!(resolve_served_version(pinned.as_ref()), expected);
}

#[test]
fn agent_card_merge_keeps_url_and_replaces_proxy_fields() {
    let merged = merge(
        r#"{"url":"https://upstream.example/a2a","protocolVersion":"0.3.0","additionalInterfaces":[{"url":"https://bypass.example"}],"securitySchemes":{"upstream":{"type":"apiKey"}},"security":[{"upstream":[]}],"supportedInterfaces":[{"url":"https://bypass.example"}],"securityRequirements":[{"upstream":[]}]}"#,
    );
    assert_eq!(merged["url"], json!("https://upstream.example/a2a"));
    assert_eq!(merged["protocolVersion"], json!("0.3"));
    assert_eq!(
        merged["supportedInterfaces"],
        value(
            r#"[{"url":"https://proxy.example/a2a/agent-1","protocolBinding":"JSONRPC","protocolVersion":"0.3"}]"#
        )
    );
    assert_eq!(
        merged["securitySchemes"],
        value(
            r#"{"LiteLLMKey":{"type":"http","scheme":"bearer","description":"LiteLLM virtual key"}}"#
        )
    );
    assert_eq!(merged["security"], value(r#"[{"LiteLLMKey":[]}]"#));
    assert!(!merged.contains_key("additionalInterfaces"));
    assert!(!merged.contains_key("securityRequirements"));
}

#[test]
fn agent_card_merge_fills_defaults_for_an_empty_card() {
    let merged = merge_agent_card(None, PROXY_URL, PROXY_BASE, None, None);
    let expected = card(
        r#"{
            "protocolVersion": "1.0",
            "version": "1.0.0",
            "capabilities": {},
            "skills": [{"id": "chat", "name": "Chat", "description": "Conversational interaction with the agent.", "tags": ["chat"]}],
            "defaultInputModes": ["text"],
            "defaultOutputModes": ["text"],
            "provider": {"organization": "LiteLLM Proxy", "url": "https://proxy.example"},
            "supportedInterfaces": [{"url": "https://proxy.example/a2a/agent-1", "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}],
            "securitySchemes": {"LiteLLMKey": {"type": "http", "scheme": "bearer", "description": "LiteLLM virtual key"}},
            "security": [{"LiteLLMKey": []}]
        }"#,
    );
    assert_eq!(merged, expected);
}

#[rstest]
#[case::null("null")]
#[case::false_value("false")]
#[case::zero("0")]
#[case::float_zero("0.0")]
#[case::empty_text("\"\"")]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
fn agent_card_merge_replaces_falsy_fields(#[case] falsy: &str) {
    let merged = merge(&format!(
        r#"{{"version":{falsy},"skills":{falsy},"defaultInputModes":{falsy},"defaultOutputModes":{falsy},"provider":{falsy}}}"#
    ));
    assert_eq!(merged["version"], json!("1.0.0"));
    assert_eq!(merged["skills"][0]["id"], json!("chat"));
    assert_eq!(merged["defaultInputModes"], json!(["text"]));
    assert_eq!(merged["defaultOutputModes"], json!(["text"]));
    assert_eq!(
        merged["provider"],
        value(r#"{"organization":"LiteLLM Proxy","url":"https://proxy.example"}"#)
    );
}

#[rstest]
#[case::true_value("true")]
#[case::number("2")]
#[case::float("0.5")]
#[case::text("\"x\"")]
#[case::list("[0]")]
#[case::object(r#"{"a":1}"#)]
fn agent_card_merge_keeps_truthy_fields(#[case] truthy: &str) {
    let merged = merge(&format!(
        r#"{{"version":{truthy},"skills":{truthy},"defaultInputModes":{truthy},"defaultOutputModes":{truthy},"provider":{truthy}}}"#
    ));
    let upstream: AgentJson = from_str(truthy).unwrap();
    assert_eq!(merged["version"], upstream);
    assert_eq!(merged["skills"], upstream);
    assert_eq!(merged["defaultInputModes"], upstream);
    assert_eq!(merged["defaultOutputModes"], upstream);
    assert_eq!(merged["provider"], upstream);
}

#[rstest]
#[case::streaming_true(r#"{"streaming":true}"#, r#"{"streaming":true}"#)]
#[case::streaming_truthy(r#"{"streaming":"yes"}"#, r#"{"streaming":"yes"}"#)]
#[case::streaming_false(r#"{"streaming":false}"#, "{}")]
#[case::streaming_zero(r#"{"streaming":0}"#, "{}")]
#[case::others_dropped(
    r#"{"streaming":1,"pushNotifications":true,"extendedAgentCard":true,"extensions":[{"uri":"x"}]}"#,
    r#"{"streaming":1}"#
)]
#[case::not_an_object(r#"[{"streaming":true}]"#, "{}")]
#[case::text("\"streaming\"", "{}")]
fn agent_card_merge_filters_capabilities(#[case] capabilities: &str, #[case] expected: &str) {
    let merged = merge(&format!(r#"{{"capabilities":{capabilities}}}"#));
    assert_eq!(merged["capabilities"], value(expected));
}

#[rstest]
#[case::both(
    Some("Configured"),
    Some("Configured text"),
    "Configured",
    "Configured text"
)]
#[case::none(None, None, "Upstream", "Original")]
#[case::empty_strings(Some(""), Some(""), "Upstream", "Original")]
#[case::name_only(Some("Configured"), None, "Configured", "Original")]
fn agent_card_merge_applies_caller_overrides(
    #[case] name: Option<&str>,
    #[case] description: Option<&str>,
    #[case] expected_name: &str,
    #[case] expected_description: &str,
) {
    let upstream = card(r#"{"name":"Upstream","description":"Original"}"#);
    let merged = merge_agent_card(Some(&upstream), PROXY_URL, PROXY_BASE, name, description);
    assert_eq!(merged["name"], json!(expected_name));
    assert_eq!(merged["description"], json!(expected_description));
}

#[test]
fn agent_card_merge_sets_overrides_on_a_card_without_them() {
    let merged = merge_agent_card(None, PROXY_URL, PROXY_BASE, Some("Named"), Some("Said"));
    assert_eq!(merged["name"], json!("Named"));
    assert_eq!(merged["description"], json!("Said"));
}

#[test]
fn agent_card_merge_keeps_only_whitelisted_fields() {
    let merged = merge(
        r#"{"name":"n","description":"d","preferredTransport":"JSONRPC","iconUrl":"i","documentationUrl":"doc","supportsAuthenticatedExtendedCard":true,"signatures":[{"protected":"p","signature":"s"}],"url":"u","unknown":"drop me","additionalInterfaces":[],"securityRequirements":[]}"#,
    );
    let keys: Vec<&str> = merged.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "capabilities",
            "defaultInputModes",
            "defaultOutputModes",
            "description",
            "documentationUrl",
            "iconUrl",
            "name",
            "preferredTransport",
            "protocolVersion",
            "provider",
            "security",
            "securitySchemes",
            "signatures",
            "skills",
            "supportedInterfaces",
            "supportsAuthenticatedExtendedCard",
            "url",
            "version",
        ]
    );
    assert_eq!(
        merged["signatures"],
        value(r#"[{"protected":"p","signature":"s"}]"#)
    );
}

#[test]
fn agent_card_merge_does_not_mutate_input() {
    let upstream = card(
        r#"{"name":"Upstream","skills":[{"id":"custom"}],"capabilities":{"streaming":false,"pushNotifications":true},"unknown":1}"#,
    );
    let original = upstream.clone();
    let merged = merge_agent_card(
        Some(&upstream),
        PROXY_URL,
        PROXY_BASE,
        Some("Configured"),
        None,
    );
    assert_eq!(upstream, original);
    assert_eq!(merged["skills"], value(r#"[{"id":"custom"}]"#));
}
