#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::mcp_env_values::{PayloadRules, StdioPolicy};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{
    ToolNameValidation, validate_and_normalize_payload, validate_mcp_server_name,
    validate_tool_display_names, validate_tool_name,
};
use crate::server_payload::request::{NewMCPServerRequest, OrderedMap, UpdateMCPServerRequest};

fn rules(separator: &str) -> PayloadRules {
    PayloadRules {
        prefix_separator: separator.to_owned(),
        stdio: StdioPolicy {
            enabled: false,
            allowed_commands: BTreeSet::new(),
        },
    }
}

fn payload(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn new_request(source: &str) -> NewMCPServerRequest {
    NewMCPServerRequest::from_value(&payload(source), &rules("-").stdio).unwrap()
}

fn update_request(source: &str) -> UpdateMCPServerRequest {
    UpdateMCPServerRequest::from_value(&payload(source), &rules("-").stdio).unwrap()
}

fn normalized(source: &str, separator: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut request = new_request(source);
    validate_and_normalize_payload(&mut request, &rules(separator))
        .map_err(|error| error.to_string())?;
    Ok((request.server_name, request.alias))
}

#[test]
fn a_separator_in_the_server_name_is_rejected() {
    assert_eq!(
        validate_mcp_server_name("bad-name", "-")
            .unwrap_err()
            .to_string(),
        "Server name cannot contain '-'. Use an alternative character instead Found: bad-name"
    );
    assert_eq!(
        validate_mcp_server_name("bad::name", "::")
            .unwrap_err()
            .to_string(),
        "Server name cannot contain '::'. Use an alternative character instead Found: bad::name"
    );
}

#[test]
fn an_empty_or_clean_name_passes() {
    assert!(validate_mcp_server_name("", "-").is_ok());
    assert!(validate_mcp_server_name("clean_name", "-").is_ok());
}

#[test]
fn the_server_name_is_checked_before_the_alias() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","server_name":"bad-name","alias":"also-bad"}"#,
            "-"
        )
        .unwrap_err(),
        "Server name cannot contain '-'. Use an alternative character instead Found: bad-name"
    );
}

#[test]
fn the_alias_is_checked_when_the_server_name_is_clean() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","server_name":"clean","alias":"bad-alias"}"#,
            "-"
        )
        .unwrap_err(),
        "Server name cannot contain '-'. Use an alternative character instead Found: bad-alias"
    );
}

#[test]
fn an_absent_alias_defaults_to_the_normalized_server_name() {
    let (server_name, alias) = normalized(
        r#"{"transport":"http","url":"u","server_name":"my_server"}"#,
        "-",
    )
    .unwrap();
    assert_eq!(server_name.as_deref(), Some("my_server"));
    assert_eq!(alias.as_deref(), Some("my_server"));
}

#[test]
fn a_space_in_the_server_name_is_normalized_into_the_alias_but_the_name_still_fails() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","server_name":"my srv"}"#,
            "-"
        )
        .unwrap_err(),
        "Invalid MCP tool prefix 'my srv' provided via server_name\nTool name contains spaces, which may cause parsing issues\nTool name contains invalid characters: ' '\nAllowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)"
    );
}

#[test]
fn an_alias_with_a_space_is_normalized_before_the_prefix_check() {
    let (server_name, alias) = normalized(
        r#"{"transport":"http","url":"u","server_name":"clean","alias":"my alias"}"#,
        "-",
    )
    .unwrap();
    assert_eq!(server_name.as_deref(), Some("clean"));
    assert_eq!(alias.as_deref(), Some("my_alias"));
}

#[test]
fn no_names_keep_the_alias_absent() {
    let (server_name, alias) = normalized(r#"{"transport":"http","url":"u"}"#, "-").unwrap();
    assert_eq!(server_name, None);
    assert_eq!(alias, None);
}

#[test]
fn updates_follow_the_same_normalization() {
    let mut request = update_request(
        r#"{"server_id":"s","transport":"http","url":"u","server_name":"my_server"}"#,
    );
    validate_and_normalize_payload(&mut request, &rules("-")).unwrap();
    assert_eq!(request.alias.as_deref(), Some("my_server"));
}

#[test]
fn an_invalid_display_name_is_rejected_with_litellms_text() {
    let mut names = OrderedMap::new();
    names.insert("tool".to_owned(), "bad name".to_owned());
    assert_eq!(
        validate_tool_display_names(&names).unwrap_err().to_string(),
        "Invalid display name 'bad name' for tool 'tool'. Display names may only contain letters, digits, underscores, and hyphens (no spaces or other special characters), since they replace the tool name sent to the LLM provider."
    );
}

#[test]
fn an_empty_display_name_is_skipped() {
    let mut names = OrderedMap::new();
    names.insert("tool".to_owned(), String::new());
    assert!(validate_tool_display_names(&names).is_ok());
}

#[test]
fn the_display_name_pattern_follows_python_dollar() {
    let mut trailing_newline = OrderedMap::new();
    trailing_newline.insert("tool".to_owned(), "abc\n".to_owned());
    assert!(validate_tool_display_names(&trailing_newline).is_ok());
}

#[rstest]
#[case::two_trailing_newlines("abc\n\n", false)]
#[case::dot("a.b", false)]
#[case::space("a b", false)]
#[case::non_ascii("café", false)]
#[case::only_a_newline("\n", false)]
#[case::plain("abc", true)]
#[case::underscore("a_b", true)]
#[case::hyphen("a-b", true)]
#[case::mixed("A1", true)]
fn the_display_name_pattern_accepts_only_its_alphabet(
    #[case] display_name: &str,
    #[case] accepted: bool,
) {
    let mut names = OrderedMap::new();
    names.insert("tool".to_owned(), display_name.to_owned());
    assert_eq!(
        validate_tool_display_names(&names).is_ok(),
        accepted,
        "{display_name}"
    );
}

#[test]
fn the_first_invalid_display_name_wins() {
    let mut names = OrderedMap::new();
    names.insert("first".to_owned(), "good".to_owned());
    names.insert("second".to_owned(), "bad name".to_owned());
    names.insert("third".to_owned(), "also bad".to_owned());
    let error = validate_tool_display_names(&names).unwrap_err();
    assert!(error.to_string().contains("for tool 'second'"));
}

#[test]
fn an_invalid_alias_prefix_names_the_alias_field() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","server_name":"clean","alias":"bad,alias"}"#,
            "-"
        )
        .unwrap_err(),
        "Invalid MCP tool prefix 'bad,alias' provided via alias\nTool name contains commas, which may cause parsing issues\nTool name contains invalid characters: ','\nAllowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)"
    );
}

#[test]
fn an_invalid_upstream_token_header_is_rejected() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","credentials":{"upstream_token_header":"bad header"}}"#,
            "-"
        )
        .unwrap_err(),
        "Invalid upstream_token_header 'bad header': must be a valid HTTP header name (RFC 7230 token, e.g. 'esb-oauth')"
    );
}

#[test]
fn a_whitespace_only_upstream_token_header_is_rejected() {
    assert_eq!(
        normalized(
            r#"{"transport":"http","url":"u","credentials":{"upstream_token_header":"  "}}"#,
            "-"
        )
        .unwrap_err(),
        "Invalid upstream_token_header '  ': must be a valid HTTP header name (RFC 7230 token, e.g. 'esb-oauth')"
    );
}

#[test]
fn an_empty_or_valid_upstream_token_header_passes() {
    assert!(
        normalized(
            r#"{"transport":"http","url":"u","credentials":{"upstream_token_header":""}}"#,
            "-"
        )
        .is_ok()
    );
    assert!(
        normalized(
            r#"{"transport":"http","url":"u","credentials":{"upstream_token_header":" esb-oauth "}}"#,
            "-"
        )
        .is_ok()
    );
}

#[test]
fn normalize_server_name_replaces_every_space() {
    let mut spaced = new_request(r#"{"server_name": "a b  c"}"#);
    assert!(validate_and_normalize_payload(&mut spaced, &rules("-")).is_err());
    assert_eq!(spaced.alias, Some("a_b__c".to_owned()));
    let mut plain = new_request(r#"{"server_name": "plain_name"}"#);
    validate_and_normalize_payload(&mut plain, &rules("-")).unwrap();
    assert_eq!(plain.alias, Some("plain_name".to_owned()));
}

#[test]
fn an_empty_tool_name_is_invalid() {
    let validation = validate_tool_name("");
    assert!(!validation.is_valid);
    assert_eq!(validation.warnings, vec!["Tool name cannot be empty"]);
}

#[test]
fn tool_name_length_counts_characters() {
    let longest = "x".repeat(128);
    assert!(validate_tool_name(&longest).is_valid);
    let too_long = "é".repeat(129);
    let validation = validate_tool_name(&too_long);
    assert!(!validation.is_valid);
    assert_eq!(
        validation.warnings,
        vec!["Tool name exceeds maximum length of 128 characters (current: 129)"]
    );
}

#[test]
fn tool_name_warnings_follow_litellms_order() {
    let validation = validate_tool_name("-a b,c.");
    assert!(!validation.is_valid);
    assert_eq!(
        validation.warnings,
        vec![
            "Tool name contains spaces, which may cause parsing issues",
            "Tool name contains commas, which may cause parsing issues",
            "Tool name starts or ends with a dash, which may cause parsing issues in some contexts",
            "Tool name starts or ends with a dot, which may cause parsing issues in some contexts",
            "Tool name contains invalid characters: ' ', ','",
            "Allowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)",
        ]
    );
}

#[test]
fn invalid_tool_name_characters_are_listed_once_in_first_seen_order() {
    let validation = validate_tool_name("a$b$c#");
    assert_eq!(
        validation.warnings,
        vec![
            "Tool name contains invalid characters: '$', '#'",
            "Allowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)",
        ]
    );
}

#[test]
fn a_conforming_tool_name_can_still_carry_warnings() {
    let validation = validate_tool_name("-a.b.");
    assert!(validation.is_valid);
    assert_eq!(
        validation.warnings,
        vec![
            "Tool name starts or ends with a dash, which may cause parsing issues in some contexts",
            "Tool name starts or ends with a dot, which may cause parsing issues in some contexts",
        ]
    );
}

#[test]
fn the_validation_result_shape_is_public() {
    let validation = ToolNameValidation {
        is_valid: true,
        warnings: Vec::new(),
    };
    assert!(validation.is_valid);
    assert!(validation.warnings.is_empty());
}
