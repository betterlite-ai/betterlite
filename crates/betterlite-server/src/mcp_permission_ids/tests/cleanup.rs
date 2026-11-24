use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::mcp_permission_ids::cleanup::{
    GrantCleanupError, NO_MCP_SERVERS, dedupe_preserving_order,
    drop_stale_object_permission_mcp_identifiers, flatten_resolved_mcp_server_ids,
};

use super::fixtures::{identifier_sets, json, string_set, strings};

fn fields(permission: JsonData) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = permission else {
        panic!("the stored grant is a JSON object");
    };
    fields
}

#[test]
fn stable_dedupe_keeps_first_occurrence() {
    assert_eq!(
        dedupe_preserving_order(&strings(&["beta", "alpha", "beta", "gamma", "alpha"])),
        strings(&["beta", "alpha", "gamma"])
    );
    assert_eq!(
        dedupe_preserving_order(&strings(&["only"])),
        strings(&["only"])
    );
    assert!(dedupe_preserving_order(&[]).is_empty());
}

#[test]
fn stored_identifiers_are_kept_not_resolved_ids() {
    let mut permission = json(
        r#"{"mcp_servers": ["alias-a", "stale", "no-mcp-servers", "alias-a", "srv-1"], "models": ["m"]}"#,
    );
    let identifiers = identifier_sets(&[
        ("alias-a", &["srv-1"]),
        ("srv-1", &["srv-1"]),
        ("stale", &[]),
    ]);
    drop_stale_object_permission_mcp_identifiers(&mut permission, &identifiers).unwrap();
    let kept = fields(permission);
    assert_eq!(
        kept.get("mcp_servers"),
        Some(&json(r#"["alias-a", "no-mcp-servers", "srv-1"]"#))
    );
    assert_eq!(kept.get("models"), Some(&json(r#"["m"]"#)));
}

#[test]
fn sentinel_survives_without_resolving() {
    let mut permission = json(r#"{"mcp_servers": ["no-mcp-servers"]}"#);
    drop_stale_object_permission_mcp_identifiers(&mut permission, &identifier_sets(&[])).unwrap();
    assert_eq!(permission, json(r#"{"mcp_servers": ["no-mcp-servers"]}"#));
    assert_eq!(NO_MCP_SERVERS, "no-mcp-servers");
}

#[test]
fn non_string_server_entries_error() {
    let mut first = json(r#"{"mcp_servers": [7, "ok"]}"#);
    let number_error =
        drop_stale_object_permission_mcp_identifiers(&mut first, &identifier_sets(&[]))
            .expect_err("a number is not a server identifier");
    assert!(matches!(
        number_error,
        GrantCleanupError::ServerIdentifier { index: 0 }
    ));
    let mut second = json(r#"{"mcp_servers": ["ok", null]}"#);
    let null_error =
        drop_stale_object_permission_mcp_identifiers(&mut second, &identifier_sets(&[]))
            .expect_err("null is not a server identifier");
    assert!(matches!(
        null_error,
        GrantCleanupError::ServerIdentifier { index: 1 }
    ));
}

#[test]
fn missing_or_wrong_shaped_servers_stay_untouched() {
    let identifiers = identifier_sets(&[("srv-a", &["id-a"])]);
    let mut absent = json(r#"{"models": ["m"]}"#);
    drop_stale_object_permission_mcp_identifiers(&mut absent, &identifiers).unwrap();
    assert_eq!(absent, json(r#"{"models": ["m"]}"#));
    let mut text = json(r#"{"mcp_servers": "srv-a"}"#);
    drop_stale_object_permission_mcp_identifiers(&mut text, &identifiers).unwrap();
    assert_eq!(text, json(r#"{"mcp_servers": "srv-a"}"#));
    let mut nothing = json(r#"{"mcp_servers": null}"#);
    drop_stale_object_permission_mcp_identifiers(&mut nothing, &identifiers).unwrap();
    assert_eq!(nothing, json(r#"{"mcp_servers": null}"#));
}

#[test]
fn stored_tool_permissions_keep_resolved_identifiers_and_dedupe_tools() {
    let mut permission = json(
        r#"{"mcp_tool_permissions": {"srv-a": ["t2", "t1", "t2"], "srv-b": "not-a-list", "srv-c": null, "stale": ["x"]}}"#,
    );
    let identifiers = identifier_sets(&[
        ("srv-a", &["id-a"]),
        ("srv-b", &["id-b"]),
        ("srv-c", &["id-c"]),
        ("stale", &[]),
    ]);
    drop_stale_object_permission_mcp_identifiers(&mut permission, &identifiers).unwrap();
    assert_eq!(
        permission,
        json(r#"{"mcp_tool_permissions": {"srv-a": ["t2", "t1"], "srv-b": [], "srv-c": []}}"#)
    );
}

#[test]
fn non_string_tool_names_error() {
    let mut permission = json(r#"{"mcp_tool_permissions": {"srv-a": ["t1", 7]}}"#);
    let identifiers = identifier_sets(&[("srv-a", &["id-a"])]);
    let error = drop_stale_object_permission_mcp_identifiers(&mut permission, &identifiers)
        .expect_err("a number is not a tool name");
    assert!(matches!(
        error,
        GrantCleanupError::ToolName { identifier } if identifier == "srv-a"
    ));
}

#[test]
fn wrong_shaped_tool_permissions_stay_untouched() {
    let identifiers = identifier_sets(&[("srv-a", &["id-a"])]);
    let mut array = json(r#"{"mcp_tool_permissions": []}"#);
    drop_stale_object_permission_mcp_identifiers(&mut array, &identifiers).unwrap();
    assert_eq!(array, json(r#"{"mcp_tool_permissions": []}"#));
    let mut text = json(r#"{"mcp_tool_permissions": "srv-a"}"#);
    drop_stale_object_permission_mcp_identifiers(&mut text, &identifiers).unwrap();
    assert_eq!(text, json(r#"{"mcp_tool_permissions": "srv-a"}"#));
}

#[test]
fn identifier_cleanup_runs_both_steps() {
    let mut permission = json(
        r#"{"mcp_servers": ["alias-a", "stale"], "mcp_tool_permissions": {"alias-a": ["t1"], "stale": ["x"]}}"#,
    );
    let identifiers = identifier_sets(&[("alias-a", &["srv-1"]), ("stale", &[])]);
    drop_stale_object_permission_mcp_identifiers(&mut permission, &identifiers).unwrap();
    assert_eq!(
        permission,
        json(r#"{"mcp_servers": ["alias-a"], "mcp_tool_permissions": {"alias-a": ["t1"]}}"#)
    );
}

#[test]
fn wrong_shaped_outer_objects_stay_untouched() {
    let identifiers = identifier_sets(&[("srv-a", &["id-a"])]);
    let mut null = json("null");
    drop_stale_object_permission_mcp_identifiers(&mut null, &identifiers).unwrap();
    assert_eq!(null, json("null"));
    let mut array = json(r#"["srv-a"]"#);
    drop_stale_object_permission_mcp_identifiers(&mut array, &identifiers).unwrap();
    assert_eq!(array, json(r#"["srv-a"]"#));
    let mut text = json(r#""srv-a""#);
    drop_stale_object_permission_mcp_identifiers(&mut text, &identifiers).unwrap();
    assert_eq!(text, json(r#""srv-a""#));
    let mut number = json("7");
    drop_stale_object_permission_mcp_identifiers(&mut number, &identifiers).unwrap();
    assert_eq!(number, json("7"));
    let mut flag = json("false");
    drop_stale_object_permission_mcp_identifiers(&mut flag, &identifiers).unwrap();
    assert_eq!(flag, json("false"));
    let mut empty = json("{}");
    drop_stale_object_permission_mcp_identifiers(&mut empty, &identifiers).unwrap();
    assert_eq!(empty, json("{}"));
}

#[test]
fn flatten_unites_every_resolved_id() {
    let resolved = identifier_sets(&[("a", &["1", "2"]), ("b", &["2", "3"]), ("c", &[])]);
    assert_eq!(
        flatten_resolved_mcp_server_ids(&resolved),
        string_set(&["1", "2", "3"])
    );
    assert!(flatten_resolved_mcp_server_ids(&identifier_sets(&[])).is_empty());
}
