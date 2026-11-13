use http::header::HeaderMap;
use rstest::rstest;

use super::support::{DEFAULT_BASE, anthropic_config, header_values, secret};
use crate::skills::config::BaseSkillsApiConfig as _;

#[test]
fn skills_anthropic_url_builds_the_default_list_endpoint() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config.get_complete_url(None, "skills", None).unwrap(),
        "https://api.anthropic.com/v1/skills"
    );
}

#[test]
fn skills_anthropic_url_prefers_the_call_api_base() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config
            .get_complete_url(Some("https://gateway.example"), "skills", None)
            .unwrap(),
        "https://gateway.example/v1/skills"
    );
}

#[test]
fn skills_anthropic_url_ignores_an_empty_call_api_base() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config.get_complete_url(Some(""), "skills", None).unwrap(),
        "https://api.anthropic.com/v1/skills"
    );
}

#[test]
fn skills_anthropic_url_concatenates_a_base_that_already_ends_in_v1() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config
            .get_complete_url(Some("https://gateway.example/anthropic/v1"), "skills", None)
            .unwrap(),
        "https://gateway.example/anthropic/v1/v1/skills"
    );
}

#[test]
fn skills_anthropic_url_encodes_the_skill_id() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config
            .get_complete_url(None, "skills", Some("skill_abc/123?x#y"))
            .unwrap(),
        "https://api.anthropic.com/v1/skills/skill_abc%2F123%3Fx%23y"
    );
}

#[test]
fn skills_anthropic_url_always_targets_the_skills_path_for_a_skill_id() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config
            .get_complete_url(None, "other", Some("skill_1"))
            .unwrap(),
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[rstest]
#[case::dot(".")]
#[case::dot_dot("..")]
fn skills_anthropic_url_rejects_dot_segments(#[case] skill_id: &str) {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let error = config
        .get_complete_url(None, "skills", Some(skill_id))
        .unwrap_err();
    assert_eq!(error.to_string(), "skill_id cannot be a dot path segment");
}

#[test]
fn skills_anthropic_url_an_empty_skill_id_lists_skills() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config.get_complete_url(None, "skills", Some("")).unwrap(),
        "https://api.anthropic.com/v1/skills"
    );
}

#[test]
fn skills_anthropic_url_get_and_delete_target_the_skill() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    assert_eq!(
        config
            .transform_get_skill_request("skill_1", Some("https://gateway.example"))
            .unwrap(),
        "https://gateway.example/v1/skills/skill_1"
    );
    assert_eq!(
        config
            .transform_delete_skill_request("skill_1", Some("https://gateway.example"))
            .unwrap(),
        "https://gateway.example/v1/skills/skill_1"
    );
}

#[test]
fn skills_anthropic_url_resolves_the_settings_api_key() {
    let config = anthropic_config(Some("sk-ant-settings"), None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, None, None)
        .unwrap();
    assert_eq!(header_values(&headers, "x-api-key"), ["sk-ant-settings"]);
    assert!(headers.get("x-api-key").unwrap().is_sensitive());
    assert_eq!(header_values(&headers, "anthropic-version"), ["2023-06-01"]);
}

#[test]
fn skills_anthropic_url_prefers_the_call_api_key() {
    let config = anthropic_config(Some("sk-ant-settings"), None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, Some(&secret("sk-ant-call")), None)
        .unwrap();
    assert_eq!(header_values(&headers, "x-api-key"), ["sk-ant-call"]);
}

#[test]
fn skills_anthropic_url_uses_the_auth_token_as_a_bearer() {
    let config = anthropic_config(None, Some("token"), DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, None, None)
        .unwrap();
    assert_eq!(header_values(&headers, "authorization"), ["Bearer token"]);
    assert!(headers.get("authorization").unwrap().is_sensitive());
}

#[test]
fn skills_anthropic_url_converts_an_oauth_key_to_a_bearer() {
    let config = anthropic_config(Some("sk-ant-oat01-abc"), None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, None, None)
        .unwrap();
    assert_eq!(
        header_values(&headers, "authorization"),
        ["Bearer sk-ant-oat01-abc"]
    );
}

#[test]
fn skills_anthropic_url_keeps_a_plain_key_on_a_custom_base_in_the_api_key_header() {
    let config = anthropic_config(None, None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(
            &mut headers,
            Some(&secret("plain-key")),
            Some("https://gateway.example"),
        )
        .unwrap();
    assert_eq!(header_values(&headers, "x-api-key"), ["plain-key"]);
    assert!(header_values(&headers, "authorization").is_empty());
}

#[test]
fn skills_anthropic_url_ignores_the_bearer_for_custom_base_setting() {
    let mut config = anthropic_config(Some("plain-key"), None, DEFAULT_BASE);
    config.settings.use_bearer_for_custom_base = true;
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, None, Some("https://gateway.example"))
        .unwrap();
    assert_eq!(header_values(&headers, "x-api-key"), ["plain-key"]);
    assert!(header_values(&headers, "authorization").is_empty());
}

#[test]
fn skills_anthropic_url_requires_credentials() {
    let config = anthropic_config(None, None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    let error = config
        .validate_environment(&mut headers, None, None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
}

#[test]
fn skills_anthropic_url_rejects_an_invalid_auth_value() {
    let config = anthropic_config(None, None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    let error = config
        .validate_environment(&mut headers, Some(&secret("bad\nkey")), None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the x-api-key header value is not a valid HTTP header value"
    );
}
