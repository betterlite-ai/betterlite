use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::json;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::forwarded_headers::merge_forwarded_headers;
use crate::api::responses_parameters::mcp_headers::{McpHeaders, extract_mcp_headers};

fn values(source: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(source).expect("a header object")
}

fn strings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn explicit_headers_win_over_client_headers_case_insensitively() {
    let merged = merge_forwarded_headers(
        Some(values(r#"{"Authorization": "new"}"#)),
        Some(strings(&[("authorization", "old"), ("x-client", "yes")])),
    );
    assert_eq!(
        merged,
        Some(values(r#"{"Authorization": "new", "x-client": "yes"}"#))
    );
}

#[test]
fn a_null_client_map_returns_the_extra_headers() {
    let extra = values(r#"{"x-test": "1"}"#);
    assert_eq!(
        merge_forwarded_headers(Some(extra.clone()), None),
        Some(extra)
    );
    assert_eq!(
        merge_forwarded_headers(Some(values(r#"{"x-test": "1"}"#)), Some(BTreeMap::new())),
        Some(values(r#"{"x-test": "1"}"#))
    );
}

#[test]
fn an_empty_extra_map_copies_the_client_headers() {
    let client = Some(strings(&[("authorization", "old")]));
    assert_eq!(
        merge_forwarded_headers(None, client.clone()),
        Some(values(r#"{"authorization": "old"}"#))
    );
    assert_eq!(
        merge_forwarded_headers(Some(BTreeMap::new()), client),
        Some(values(r#"{"authorization": "old"}"#))
    );
}

#[test]
fn no_headers_at_all_stay_null() {
    assert_eq!(merge_forwarded_headers(None, None), None);
}

fn mcp_headers(
    secret_fields: Option<&JsonValue>,
    tools: Option<&[JsonValue]>,
) -> Result<McpHeaders, ResponsesParametersError> {
    extract_mcp_headers(secret_fields, tools, "x-mcp-auth")
}

fn exposed_server(headers: &McpHeaders, alias: &str, name: &str) -> Option<String> {
    headers
        .server_auth_headers
        .as_ref()
        .and_then(|servers| servers.get(alias))
        .and_then(|entries| entries.get(name))
        .map(|secret| secret.expose_secret().to_owned())
}

fn exposed_oauth(headers: &McpHeaders) -> Option<String> {
    headers
        .oauth2_headers
        .as_ref()
        .and_then(|entries| entries.get("Authorization"))
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn raw_headers_supply_the_legacy_server_and_oauth_outputs() {
    let secret_fields = json(
        r#"{"raw_headers": {"x-mcp-auth": "legacy", "x-mcp-github-authorization": "old", "Authorization": "oauth"}}"#,
    );
    let headers = mcp_headers(Some(&secret_fields), None).expect("raw headers parse");
    assert_eq!(
        headers
            .auth_header
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("legacy".to_owned())
    );
    assert_eq!(
        exposed_server(&headers, "github", "Authorization"),
        Some("old".to_owned())
    );
    assert_eq!(exposed_oauth(&headers), Some("oauth".to_owned()));
    assert_eq!(
        headers.raw_headers,
        Some(strings(&[
            ("Authorization", "oauth"),
            ("x-mcp-auth", "legacy"),
            ("x-mcp-github-authorization", "old"),
        ]))
    );
}

#[test]
fn tool_headers_overwrite_but_do_not_recompute_the_other_outputs() {
    let secret_fields = json(
        r#"{"raw_headers": {"x-mcp-auth": "legacy", "x-mcp-github-authorization": "old", "Authorization": "oauth"}}"#,
    );
    let tools = [json(
        r#"{"type": "mcp", "headers": {"x-mcp-github-authorization": "new", "x-custom": "v"}}"#,
    )];
    let headers = mcp_headers(Some(&secret_fields), Some(&tools)).expect("tool headers parse");
    assert_eq!(
        headers
            .auth_header
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("legacy".to_owned())
    );
    assert_eq!(
        exposed_server(&headers, "github", "Authorization"),
        Some("new".to_owned())
    );
    assert_eq!(exposed_oauth(&headers), Some("oauth".to_owned()));
    assert_eq!(
        headers.raw_headers,
        Some(strings(&[
            ("Authorization", "oauth"),
            ("x-custom", "v"),
            ("x-mcp-auth", "legacy"),
            ("x-mcp-github-authorization", "new"),
        ]))
    );
}

#[test]
fn a_tool_can_supply_server_headers_without_raw_headers() {
    let tools = [json(
        r#"{"type": "mcp", "headers": {"x-mcp-github-authorization": "token"}}"#,
    )];
    let headers = mcp_headers(None, Some(&tools)).expect("tool headers parse");
    assert!(headers.auth_header.is_none());
    assert_eq!(
        exposed_server(&headers, "github", "Authorization"),
        Some("token".to_owned())
    );
    assert!(headers.oauth2_headers.is_none());
    assert_eq!(
        headers.raw_headers,
        Some(strings(&[("x-mcp-github-authorization", "token")]))
    );
}

#[test]
fn a_tool_header_name_is_matched_case_insensitively() {
    let tools = [json(
        r#"{"type": "mcp", "headers": {"X-MCP-GitHub-Authorization": "token"}}"#,
    )];
    let headers = mcp_headers(None, Some(&tools)).expect("tool headers parse");
    assert_eq!(
        exposed_server(&headers, "github", "Authorization"),
        Some("token".to_owned())
    );
    assert_eq!(
        headers.raw_headers,
        Some(strings(&[("X-MCP-GitHub-Authorization", "token")]))
    );
}

#[rstest]
#[case::empty_headers(r#"{"type": "mcp", "headers": {}}"#)]
#[case::no_headers(r#"{"type": "mcp"}"#)]
#[case::non_mcp(r#"{"type": "function", "headers": {"x-mcp-github-authorization": "token"}}"#)]
#[case::non_dict_headers(r#"{"type": "mcp", "headers": "token"}"#)]
#[case::non_dict_tool("3")]
fn tool_headers_that_do_not_apply_are_ignored(#[case] source: &str) {
    let tools = [json(source)];
    let headers = mcp_headers(None, Some(&tools)).expect("ignored tools do not fail");
    assert!(headers.auth_header.is_none());
    assert!(headers.server_auth_headers.is_none());
    assert!(headers.oauth2_headers.is_none());
    assert!(headers.raw_headers.is_none());
}

#[test]
fn no_raw_headers_and_no_tools_produce_four_nulls() {
    let headers = mcp_headers(None, None).expect("nothing to read");
    assert!(headers.auth_header.is_none());
    assert!(headers.server_auth_headers.is_none());
    assert!(headers.oauth2_headers.is_none());
    assert!(headers.raw_headers.is_none());
    let empty = json(r#"{"raw_headers": {}}"#);
    let empty_headers = mcp_headers(Some(&empty), None).expect("empty raw headers");
    assert!(empty_headers.server_auth_headers.is_none());
    assert!(empty_headers.oauth2_headers.is_none());
    assert_eq!(empty_headers.raw_headers, Some(BTreeMap::new()));
}

#[test]
fn a_secret_fields_object_without_raw_headers_is_ignored() {
    let secret_fields = json(r#"{"other": 1}"#);
    let headers = mcp_headers(Some(&secret_fields), None).expect("no raw headers");
    assert!(headers.raw_headers.is_none());
}

#[test]
fn duplicate_authorization_headers_are_rejected() {
    let secret_fields = json(r#"{"raw_headers": {"Authorization": "a", "authorization": "b"}}"#);
    assert!(matches!(
        mcp_headers(Some(&secret_fields), None),
        Err(ResponsesParametersError::DuplicateAuthorization { .. })
    ));
}

#[test]
fn a_non_string_tool_header_value_is_an_error() {
    let tools = [json(
        r#"{"type": "mcp", "headers": {"x-mcp-github-authorization": 3}}"#,
    )];
    assert!(matches!(
        mcp_headers(None, Some(&tools)),
        Err(ResponsesParametersError::NotText {
            field: "tools.headers"
        })
    ));
}
