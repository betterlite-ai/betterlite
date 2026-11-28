#![cfg(test)]

use super::{FALLBACK_SERVER_NAME, ServerMetadata, resolve, server_initial};
use crate::byok_consent::test_support::{registry, server};
use crate::server_record::MCPServer;

fn github() -> MCPServer {
    server(
        r#"{"server_id": "srv-1", "name": "github", "server_name": "GitHub", "transport": "sse", "byok_description": ["Read repositories", "Open pull requests"], "byok_api_key_help_url": "https://github.com/settings/tokens"}"#,
    )
}

#[test]
fn the_fallback_name_is_mcp_server() {
    assert_eq!(FALLBACK_SERVER_NAME, "MCP Server");
}

#[test]
fn a_missing_server_id_falls_back() {
    assert_eq!(
        resolve(None, Some(&registry(vec![github()]))),
        ServerMetadata {
            server_name: "MCP Server".to_owned(),
            access_items: Vec::new(),
            help_url: String::new(),
        }
    );
}

#[test]
fn an_unknown_server_id_falls_back() {
    let resolved = resolve(Some("srv-missing"), Some(&registry(vec![github()])));
    assert_eq!(resolved.server_name, "MCP Server");
    assert!(resolved.access_items.is_empty());
    assert!(resolved.help_url.is_empty());
}

#[test]
fn a_registry_entry_supplies_the_metadata() {
    let resolved = resolve(Some("srv-1"), Some(&registry(vec![github()])));
    assert_eq!(
        resolved,
        ServerMetadata {
            server_name: "GitHub".to_owned(),
            access_items: vec![
                "Read repositories".to_owned(),
                "Open pull requests".to_owned()
            ],
            help_url: "https://github.com/settings/tokens".to_owned(),
        }
    );
}

#[test]
fn the_display_name_falls_back_to_the_name() {
    let plain = server(r#"{"server_id": "srv-2", "name": "notion", "transport": "sse"}"#);
    let resolved = resolve(Some("srv-2"), Some(&registry(vec![plain])));
    assert_eq!(resolved.server_name, "notion");
}

#[test]
fn an_empty_display_name_falls_back_to_the_name() {
    let plain = server(
        r#"{"server_id": "srv-2", "name": "notion", "server_name": "", "transport": "sse"}"#,
    );
    let resolved = resolve(Some("srv-2"), Some(&registry(vec![plain])));
    assert_eq!(resolved.server_name, "notion");
}

#[test]
fn a_missing_help_url_is_empty() {
    let plain = server(r#"{"server_id": "srv-2", "name": "notion", "transport": "sse"}"#);
    let resolved = resolve(Some("srv-2"), Some(&registry(vec![plain])));
    assert_eq!(resolved.help_url, "");
    assert!(resolved.access_items.is_empty());
}

#[test]
fn a_registry_without_the_server_falls_back() {
    let resolved = resolve(Some("srv-1"), Some(&registry(Vec::new())));
    assert_eq!(resolved.server_name, "MCP Server");
}

#[test]
fn the_initial_is_the_uppercased_first_letter() {
    assert_eq!(server_initial("github"), "G");
    assert_eq!(server_initial("GitHub"), "G");
    assert_eq!(server_initial("notion"), "N");
}

#[test]
fn an_empty_name_has_the_initial_s() {
    assert_eq!(server_initial(""), "S");
}

#[test]
fn a_non_ascii_first_letter_is_uppercased() {
    assert_eq!(server_initial("éclair"), "É");
}
