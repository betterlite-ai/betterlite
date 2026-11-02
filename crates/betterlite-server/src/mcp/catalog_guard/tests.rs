#![cfg(test)]

use std::collections::BTreeMap;

use super::description_overrides::apply_description_overrides;
use super::pin::pin_tool_catalog;
use super::prefix::{PrefixPolicy, strip_known_server_prefix};
use super::types::{MCPServer, MCPTool};
use super::{CatalogAlert, PinnedCatalogDrift};
use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};

fn tool(name: &str, description: Option<&str>) -> MCPTool {
    MCPTool {
        name: name.to_owned(),
        description: description.map(str::to_owned),
        input_schema: BTreeMap::new(),
    }
}

fn pinned(description: &str) -> PinnedMCPTool {
    let source = format!(r#"{{"description":"{description}","input_schema":{{}}}}"#);
    serde_json::from_str(&source).expect("valid pinned tool")
}

fn server() -> MCPServer {
    MCPServer {
        server_id: "server-id".to_owned(),
        name: "server".to_owned(),
        alias: None,
        server_name: None,
        short_prefix: None,
        extra_headers: Vec::new(),
        tool_name_to_description: BTreeMap::new(),
    }
}

#[test]
fn catalog_alert_types() {
    let alert = CatalogAlert {
        signature: "a".to_owned(),
        message: "m".to_owned(),
    };
    assert_eq!(alert.signature, "a");
    assert_eq!(alert.message, "m");
}

#[test]
fn drift_alert_is_absent_when_empty() {
    let drift = PinnedCatalogDrift {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    };
    assert_eq!(drift.alert(&server()), None);
}

#[test]
fn drift_alert_uses_pipe_signature_and_labels() {
    let drift = PinnedCatalogDrift {
        added: vec!["new".to_owned()],
        removed: vec!["old".to_owned()],
        changed: vec!["changed".to_owned()],
    };
    let alert = drift.alert(&server()).expect("non-empty drift alerts");
    assert_eq!(
        alert.signature,
        "added: `new`|removed: `old`|changed: `changed`"
    );
    assert!(alert.message.contains("serving the pinned tools"));
}

#[test]
fn pin_intersection_restores_changed_description() {
    let upstream = vec![tool("kept", Some("new")), tool("added", Some("x"))];
    let mut pins = BTreeMap::new();
    pins.insert("kept".to_owned(), pinned("old"));
    pins.insert("removed".to_owned(), pinned("gone"));
    let (served, drift) = pin_tool_catalog(&upstream, &pins);
    assert_eq!(served, vec![tool("kept", Some("old"))]);
    let catalog_drift = drift.expect("catalog changes alert");
    assert_eq!(catalog_drift.added, vec!["added"]);
    assert_eq!(catalog_drift.removed, vec!["removed"]);
    assert_eq!(catalog_drift.changed, vec!["kept"]);
}

#[test]
fn override_uses_stripped_tool_name() {
    let mut configured = server();
    configured.alias = Some("team-server".to_owned());
    configured
        .tool_name_to_description
        .insert("search".to_owned(), "safe".to_owned());
    let output = apply_description_overrides(
        &[tool("team-server-search", Some("old"))],
        &configured,
        &PrefixPolicy::default(),
    );
    assert_eq!(output[0].description.as_deref(), Some("safe"));
}

#[test]
fn strip_prefix_longest_match_and_fallbacks() {
    let mut configured = server();
    configured.server_id = "server-id".to_owned();
    configured.alias = Some("server".to_owned());
    let policy = PrefixPolicy::default();
    assert_eq!(
        strip_known_server_prefix("server-id-tool", Some(&configured), &policy),
        "tool"
    );
    assert_eq!(
        strip_known_server_prefix("bare-tool", Some(&configured), &policy),
        "bare-tool"
    );
    assert_eq!(strip_known_server_prefix("one-tool", None, &policy), "tool");
}

#[test]
fn pin_empty_catalog_has_no_drift() {
    let (served, drift) = pin_tool_catalog(&[], &BTreeMap::new());
    assert!(served.is_empty());
    assert!(drift.is_none());
}

#[test]
fn json_value_can_represent_schema() {
    let value = JsonValue::Object(BTreeMap::from([(
        String::from("type"),
        JsonValue::String(String::from("object")),
    )]));
    assert!(matches!(value, JsonValue::Object(_)));
}
