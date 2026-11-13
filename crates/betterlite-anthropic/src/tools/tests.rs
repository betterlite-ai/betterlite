#![cfg(test)]

mod tools_cases;

use std::slice;

use betterlite_prompts::arguments::json::Json;

use super::map_tools;
use super::wire::McpServerTool;

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn first_server(tool: &Json) -> McpServerTool {
    let mapping = map_tools(slice::from_ref(tool)).unwrap();
    mapping.mcp_servers.into_iter().next().unwrap()
}

fn mcp_token(server: &McpServerTool) -> Option<String> {
    match server {
        McpServerTool::Mapped(mapped) => mapped.authorization_token.clone(),
        McpServerTool::Descriptor(fields) => fields
            .get("authorization_token")
            .and_then(Json::as_str)
            .map(str::to_owned),
    }
}

fn mcp_source(identity: &str, token: &str, other_header: &str) -> String {
    format!(
        r#"{{"type":"mcp","server_url":"https://mcp.example.test/rpc","server_label":"{identity}","headers":{{"Authorization":"{token}","X-Api-Key":"{other_header}"}}}}"#
    )
}

fn identity_server(identity: &str, token: &str, other_header: &str) -> McpServerTool {
    first_server(&json(&mcp_source(identity, token, other_header)))
}

#[test]
fn tools_credential_identity_isolation() {
    let first = identity_server("identity-a", "Bearer alpha-token", "alpha-header");
    let second = identity_server("identity-b", "Bearer beta-token", "beta-header");
    assert_eq!(mcp_token(&first), Some("alpha-token".to_owned()));
    assert_eq!(mcp_token(&second), Some("beta-token".to_owned()));
    let first_text = serde_json::to_string(&first).unwrap();
    let second_text = serde_json::to_string(&second).unwrap();
    assert!(!first_text.contains("beta-token"));
    assert!(!first_text.contains("beta-header"));
    assert!(!second_text.contains("alpha-token"));
    assert!(!second_text.contains("alpha-header"));
    // a second conversion of the first identity still yields its own credential, so nothing was cached per tool text
    let again = identity_server("identity-a", "Bearer alpha-token", "alpha-header");
    assert_eq!(mcp_token(&again), Some("alpha-token".to_owned()));
    assert!(
        !serde_json::to_string(&again)
            .unwrap()
            .contains("beta-token")
    );
}

const FIXTURE_SECRET: &str = "sk-ant-fixture-9c1e27b4d05a4f6e8b3d";

fn secret_tool() -> Json {
    json(&format!(
        r#"{{"type":"mcp","server_url":"https://mcp.example.test/rpc","server_label":"fixture","headers":{{"Authorization":"Bearer {FIXTURE_SECRET}"}}}}"#
    ))
}

fn assert_error_hides_secret(tool: &Json) {
    let Err(failure) = map_tools(slice::from_ref(tool)) else {
        panic!("expected a mapping failure");
    };
    assert!(!failure.to_string().contains(FIXTURE_SECRET));
    assert!(!format!("{failure:?}").contains(FIXTURE_SECRET));
}

#[test]
fn tools_secret_safe_failure() {
    // the owned credential redacts itself, and no Debug of the wire descriptor exists to print it
    let tool = secret_tool();
    let fields = tool.as_object().unwrap();
    let token = super::mcp::authorization_token(fields)
        .unwrap()
        .expect("the fixture carries a token");
    let debug = format!("{token:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains(FIXTURE_SECRET));
    let server = first_server(&tool);
    assert!(
        serde_json::to_string(&server)
            .unwrap()
            .contains(FIXTURE_SECRET)
    );

    let broken = json(&format!(
        r#"{{"type":"mcp","server_url":"https://mcp.example.test/rpc","server_label":"fixture","allowed_tools":"not-a-list","headers":{{"Authorization":"Bearer {FIXTURE_SECRET}"}}}}"#
    ));
    assert_error_hides_secret(&broken);
    let unknown = json(&format!(
        r#"{{"type":"unknown_tool","headers":{{"Authorization":"Bearer {FIXTURE_SECRET}"}}}}"#
    ));
    assert_error_hides_secret(&unknown);
}
