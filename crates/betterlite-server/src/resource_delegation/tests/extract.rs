use std::collections::BTreeSet;

use crate::mcp_permission_ids::team::ALL_PROXY_MCP_SERVERS;
use crate::resource_delegation::requested::{requested_entries, requested_server_ids};

use super::fixtures::{json, string_set, strings};

fn server_ids(text: &str) -> BTreeSet<String> {
    requested_server_ids(Some(&json(text))).unwrap()
}

fn groups(text: &str) -> BTreeSet<String> {
    requested_entries("mcp_access_groups", Some(&json(text)), false).unwrap()
}

fn toolsets(text: &str) -> BTreeSet<String> {
    requested_entries("mcp_toolsets", Some(&json(text)), false).unwrap()
}

fn vector_stores(text: &str) -> BTreeSet<String> {
    requested_entries("vector_stores", Some(&json(text)), true).unwrap()
}

fn search_tools(text: &str) -> Vec<String> {
    requested_entries("search_tools", Some(&json(text)), true).unwrap()
}

#[test]
fn extractor_falsey_shapes() {
    assert!(requested_server_ids(None).unwrap().is_empty());
    assert!(server_ids("null").is_empty());
    assert!(server_ids("{}").is_empty());
    assert!(server_ids("[]").is_empty());
    assert!(server_ids(r#""mcp_servers""#).is_empty());
    assert!(server_ids(r#"{"mcp_servers": null}"#).is_empty());
    assert!(server_ids(r#"{"mcp_servers": {}}"#).is_empty());
    assert!(server_ids(r#"{"mcp_tool_permissions": []}"#).is_empty());
    assert!(groups(r#"{"mcp_access_groups": "group-a"}"#).is_empty());
    assert!(toolsets(r#"{"mcp_toolsets": {}}"#).is_empty());
    assert!(vector_stores(r#"{"vector_stores": "v"}"#).is_empty());
    assert!(search_tools(r#"{"search_tools": null}"#).is_empty());
}

#[test]
fn extractor_contents() {
    assert_eq!(
        server_ids(
            r#"{"mcp_servers": ["no-mcp-servers", "srv-a", "srv-a"], "mcp_tool_permissions": {"no-mcp-servers": [], "srv-b": ["t"]}}"#
        ),
        string_set(&["no-mcp-servers", "srv-a", "srv-b"])
    );
    assert_eq!(
        server_ids(r#"{"mcp_servers": ["all-proxy-mcpservers"]}"#),
        string_set(&[ALL_PROXY_MCP_SERVERS])
    );
    assert_eq!(
        groups(r#"{"mcp_access_groups": ["", "group-a", "group-a"]}"#),
        string_set(&["", "group-a"])
    );
    assert_eq!(
        toolsets(r#"{"mcp_toolsets": ["", "t-a"]}"#),
        string_set(&["", "t-a"])
    );
    assert_eq!(
        vector_stores(r#"{"vector_stores": ["b", "", "a", "b"]}"#),
        string_set(&["a", "b"])
    );
    assert_eq!(
        search_tools(r#"{"search_tools": ["b", "", "a", "b"]}"#),
        strings(&["b", "a", "b"])
    );
}

#[test]
fn extractor_entry_type_errors() {
    let server_error =
        requested_server_ids(Some(&json(r#"{"mcp_servers": ["srv-a", null]}"#))).unwrap_err();
    assert_eq!(
        server_error.to_string(),
        "mcp_servers entry 1 is not a string"
    );
    let tool_error = requested_entries::<Vec<String>>(
        "search_tools",
        Some(&json(r#"{"search_tools": [7]}"#)),
        true,
    )
    .unwrap_err();
    assert_eq!(
        tool_error.to_string(),
        "search_tools entry 0 is not a string"
    );
    let store_error = requested_entries::<BTreeSet<String>>(
        "vector_stores",
        Some(&json(r#"{"vector_stores": [{}]}"#)),
        true,
    )
    .unwrap_err();
    assert_eq!(
        store_error.to_string(),
        "vector_stores entry 0 is not a string"
    );
}
