#![cfg(test)]

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;
use secrecy::SecretString;

use super::compute_user_env_status;
use crate::mcp_user_env_routes::types::{MCPUserEnvVarSpec, MCPUserEnvVarsStatus};

const TOKEN_HEADER: &str = r#"{"Authorization":"Bearer ${TOKEN}"}"#;
const TOKEN_DECLARATION: &str =
    r#"[{"name":"TOKEN","scope":"user","description":"Paste the token"}]"#;
const TOKEN_AND_UNUSED: &str = r#"[{"name":"TOKEN","scope":"user","description":"Paste the token"},{"name":"UNUSED","scope":"user"}]"#;

fn json_option(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), |text| format!(r#""{text}""#))
}

fn parse_table(
    server_id: &str,
    name: Option<&str>,
    alias: Option<&str>,
    env_vars: &str,
    static_headers: &str,
) -> McpServerTable {
    let raw = format!(
        r#"{{"server_id":"{server_id}","server_name":{name},"alias":{alias},"transport":"http","env_vars":{env_vars},"static_headers":{static_headers}}}"#,
        name = json_option(name),
        alias = json_option(alias),
    );
    serde_json::from_str(&raw).unwrap()
}

fn server(env_vars: &str, static_headers: &str) -> McpServerTable {
    parse_table(
        "srv-1",
        Some("Server One"),
        Some("one"),
        env_vars,
        static_headers,
    )
}

fn stored(pairs: &[(&str, &str)]) -> UserEnvVars {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), SecretString::from(*value)))
        .collect()
}

fn required_names(status: &MCPUserEnvVarsStatus) -> Vec<&str> {
    status
        .required
        .iter()
        .map(|spec| spec.name.as_str())
        .collect()
}

#[test]
fn a_referenced_variable_without_a_global_is_required() {
    let table = server(TOKEN_DECLARATION, TOKEN_HEADER);
    let status = compute_user_env_status(
        &table,
        &stored(&[("TOKEN", "secret")]),
        Some("https://proxy.example"),
    )
    .unwrap();
    assert_eq!(required_names(&status), ["TOKEN"]);
    assert_eq!(
        status.required,
        [MCPUserEnvVarSpec {
            name: "TOKEN".to_owned(),
            description: Some("Paste the token".to_owned()),
            is_set: true,
        }]
    );
    assert_eq!(status.missing_count, 0);
    assert_eq!(
        status.setup_url.as_deref(),
        Some("https://proxy.example/ui/mcp-servers?fill_env_vars=srv-1")
    );
    assert_eq!(status.server_id, "srv-1");
    assert_eq!(status.server_name.as_deref(), Some("Server One"));
    assert_eq!(status.alias.as_deref(), Some("one"));
}

#[test]
fn a_blank_stored_value_reports_unset() {
    let table = server(TOKEN_DECLARATION, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[("TOKEN", "")]), None).unwrap();
    assert_eq!(
        status.required,
        [MCPUserEnvVarSpec {
            name: "TOKEN".to_owned(),
            description: Some("Paste the token".to_owned()),
            is_set: false,
        }]
    );
    assert_eq!(status.missing_count, 1);
}

#[test]
fn an_absent_stored_value_reports_unset() {
    let table = server(TOKEN_DECLARATION, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert_eq!(required_names(&status), ["TOKEN"]);
    assert!(!status.required[0].is_set);
    assert_eq!(status.missing_count, 1);
}

#[test]
fn a_user_variable_no_header_references_is_absent() {
    let table = server(TOKEN_AND_UNUSED, TOKEN_HEADER);
    let status = compute_user_env_status(
        &table,
        &stored(&[("TOKEN", "secret"), ("UNUSED", "stored")]),
        None,
    )
    .unwrap();
    assert_eq!(required_names(&status), ["TOKEN"]);
    assert_eq!(status.missing_count, 0);
}

#[test]
fn an_empty_global_does_not_cover_a_user_variable() {
    let declaration = r#"[{"name":"TOKEN","scope":"user","description":"Paste the token"},{"name":"TOKEN","scope":"global","value":""}]"#;
    let table = server(declaration, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert_eq!(required_names(&status), ["TOKEN"]);
    assert_eq!(status.missing_count, 1);
    assert!(status.setup_url.is_some());
}

#[test]
fn a_non_empty_global_covers_a_user_variable() {
    let declaration =
        r#"[{"name":"TOKEN","scope":"user"},{"name":"TOKEN","scope":"global","value":"fallback"}]"#;
    let table = server(declaration, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert!(status.required.is_empty());
    assert_eq!(status.missing_count, 0);
    assert!(status.setup_url.is_none());
}

#[test]
fn required_variables_follow_the_declaration_order() {
    let declaration = r#"[{"name":"ZED","scope":"user"},{"name":"ALPHA","scope":"user"}]"#;
    let headers = r#"{"X-Zed":"${ZED}","X-Alpha":"${ALPHA}"}"#;
    let table = server(declaration, headers);
    let status = compute_user_env_status(&table, &stored(&[("ALPHA", "set")]), None).unwrap();
    assert_eq!(required_names(&status), ["ZED", "ALPHA"]);
    assert_eq!(status.missing_count, 1);
}

#[test]
fn a_server_without_env_vars_requires_nothing() {
    let table = server("null", TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert!(status.required.is_empty());
    assert!(status.setup_url.is_none());
}

#[test]
fn a_server_without_static_headers_requires_nothing() {
    let table = server(TOKEN_DECLARATION, "null");
    let status = compute_user_env_status(&table, &stored(&[("TOKEN", "secret")]), None).unwrap();
    assert!(status.required.is_empty());
    assert!(status.setup_url.is_none());
}

#[test]
fn a_server_without_identity_fields_reports_none() {
    let table = parse_table("srv-1", None, None, TOKEN_DECLARATION, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert!(status.server_name.is_none());
    assert!(status.alias.is_none());
}

#[test]
fn the_setup_url_percent_encodes_the_server_id_and_trims_the_base() {
    let table = parse_table(
        "srv/one",
        Some("Server One"),
        None,
        TOKEN_DECLARATION,
        TOKEN_HEADER,
    );
    let status =
        compute_user_env_status(&table, &stored(&[]), Some("https://proxy.example/")).unwrap();
    assert_eq!(
        status.setup_url.as_deref(),
        Some("https://proxy.example/ui/mcp-servers?fill_env_vars=srv%2Fone")
    );
}

#[test]
fn the_setup_url_is_a_path_without_a_configured_base() {
    let table = server(TOKEN_DECLARATION, TOKEN_HEADER);
    let status = compute_user_env_status(&table, &stored(&[]), None).unwrap();
    assert_eq!(
        status.setup_url.as_deref(),
        Some("/ui/mcp-servers?fill_env_vars=srv-1")
    );
}

#[test]
fn the_status_never_carries_a_stored_value() {
    let table = server(TOKEN_DECLARATION, TOKEN_HEADER);
    let status =
        compute_user_env_status(&table, &stored(&[("TOKEN", "sk-live-secret")]), None).unwrap();
    let rendered = serde_json::to_string(&status).unwrap();
    assert!(rendered.contains(r#""is_set":true"#));
    assert!(!rendered.contains("sk-live-secret"));
}
