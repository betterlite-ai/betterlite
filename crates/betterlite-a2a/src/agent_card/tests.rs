#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use serde_json::{from_str, to_value};

use super::{
    AgentCard, merge_agent_card, normalize_protocol_version, normalize_protocol_version_bytes,
    resolve_served_version,
};

const PROXY_URL: &str = "https://proxy.example/a2a/agent-1";
const PROXY_BASE: &str = "https://proxy.example";

fn card(text: &str) -> AgentCard {
    from_str(text).expect("object card")
}
fn value(text: &str) -> AgentJson {
    from_str(text).expect("JSON value")
}
fn merged(text: &str) -> AgentJson {
    to_value(merge_agent_card(
        Some(&card(text)),
        PROXY_URL,
        PROXY_BASE,
        None,
        None,
    ))
    .expect("card JSON")
}

#[rstest]
#[case("\"0.3\"", Some("0.3"))]
#[case("\"1.0\"", Some("1.0"))]
#[case("\"0.3.0-rc1+build.42\"", Some("0.3"))]
#[case("\"1.0.1+build.42\"", Some("1.0"))]
#[case("\"1.0\\n\"", Some("1.0"))]
#[case("\"1.0\\n\\n\"", None)]
#[case("\"01.0\"", None)]
#[case("\"1.0-rc1\"", None)]
#[case("1.0", None)]
fn normalizes_versions(#[case] raw: &str, #[case] expected: Option<&str>) {
    assert_eq!(normalize_protocol_version(&value(raw)), expected);
}

#[test]
fn resolves_invalid_or_absent_versions_to_default() {
    assert_eq!(
        resolve_served_version(Some(&card(r#"{"protocolVersion":"bad"}"#))),
        "1.0"
    );
    assert_eq!(resolve_served_version(None), "1.0");
}

#[test]
fn merge_replaces_proxy_fields_and_drops_bypass_fields() {
    let result = merged(
        r#"{"url":"https://upstream","protocolVersion":"0.3.0","additionalInterfaces":[{}],"securitySchemes":{"upstream":{"type":"apiKey"}},"security":[{"upstream":[]}],"unknown":true}"#,
    );
    assert_eq!(result["url"], value(r#""https://upstream""#));
    assert_eq!(result["protocolVersion"], value(r#""0.3""#));
    assert_eq!(
        result["supportedInterfaces"][0]["url"],
        value(r#""https://proxy.example/a2a/agent-1""#)
    );
    assert_eq!(
        result["securitySchemes"]["LiteLLMKey"]["scheme"],
        value(r#""bearer""#)
    );
    assert!(
        !result
            .as_object()
            .expect("object")
            .contains_key("additionalInterfaces")
    );
    assert!(!result.as_object().expect("object").contains_key("unknown"));
}

#[test]
fn merge_fills_defaults_and_honors_overrides() {
    let result = to_value(merge_agent_card(
        None,
        PROXY_URL,
        PROXY_BASE,
        Some("Named"),
        Some("Said"),
    ))
    .expect("card JSON");
    assert_eq!(result["name"], value(r#""Named""#));
    assert_eq!(result["description"], value(r#""Said""#));
    assert_eq!(result["version"], value(r#""1.0.0""#));
    assert_eq!(result["skills"][0]["id"], value(r#""chat""#));
    assert_eq!(result["defaultInputModes"], value(r#"["text"]"#));
    assert_eq!(result["defaultOutputModes"], value(r#"["text"]"#));
}

#[test]
fn merge_preserves_truthy_upstream_values() {
    let result = merged(
        r#"{"version":"custom","skills":[{"id":"x"}],"defaultInputModes":["audio"],"defaultOutputModes":["audio"],"provider":{"organization":"x","url":"u"}}"#,
    );
    assert_eq!(result["version"], value(r#""custom""#));
    assert_eq!(result["defaultInputModes"], value(r#"["audio"]"#));
    assert_eq!(result["provider"]["organization"], value(r#""x""#));
}

#[rstest]
#[case(r#"{"streaming":true}"#, r#"{"streaming":true}"#)]
#[case(r#"{"streaming":"yes"}"#, r#"{"streaming":"yes"}"#)]
#[case(r#"{"streaming":false,"pushNotifications":true}"#, "{}")]
#[case(r#"[{"streaming":true}]"#, "{}")]
fn filters_capabilities(#[case] input: &str, #[case] expected: &str) {
    let result = merged(&format!(r#"{{"capabilities":{input}}}"#));
    assert_eq!(result["capabilities"], value(expected));
}

#[test]
fn large_nonzero_numbers_are_truthy_without_f64_narrowing() {
    let huge = "1".repeat(401);
    let result = merged(&format!(
        r#"{{"version":{huge},"capabilities":{{"streaming":{huge}}}}}"#
    ));
    let expected: AgentJson = value(&huge);
    assert_eq!(result["version"], expected);
    assert_eq!(result["capabilities"]["streaming"], expected);
}

#[test]
fn card_input_is_not_mutated() {
    let input =
        card(r#"{"url":"upstream","skills":[{"id":"custom"}],"capabilities":{"streaming":false}}"#);
    let original = to_value(&input).expect("card JSON");
    drop(merge_agent_card(
        Some(&input),
        PROXY_URL,
        PROXY_BASE,
        None,
        None,
    ));
    assert_eq!(to_value(input).expect("card JSON"), original);
}

#[test]
fn replays_protocol_version_fuzz_corpus() {
    let corpus = include_bytes!("../../fuzz/corpus/normalize_protocol_version/semver");
    assert!(normalize_protocol_version_bytes(corpus).is_some());
}
