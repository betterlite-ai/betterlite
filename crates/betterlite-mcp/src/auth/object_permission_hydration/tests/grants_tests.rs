use std::collections::BTreeMap;

use super::super::error::EntitlementError;
use super::super::grants::{
    key_or_team_declares_toolsets, toolset_tool_permissions, toolset_tools_for_server,
    union_tool_grants,
};
use super::super::record::{MCP_ALL_TOOLS_WILDCARD, McpAuth};
use super::{FakeReads, Scripted, expected, grants_of, mcp_auth, row, row_with_toolsets};

#[tokio::test]
async fn toolset_grants_are_empty_without_a_row_or_declared_toolsets() {
    let reads = FakeReads::standard();
    assert_eq!(
        toolset_tool_permissions(&reads, None, false).await.unwrap(),
        BTreeMap::new()
    );
    assert_eq!(
        toolset_tool_permissions(&reads, Some(&row("op")), false)
            .await
            .unwrap(),
        BTreeMap::new()
    );
    assert_eq!(
        toolset_tool_permissions(&reads, Some(&row_with_toolsets("op", &[])), false)
            .await
            .unwrap(),
        BTreeMap::new()
    );
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn toolset_grants_resolve_the_declared_ids_with_the_fresh_flag() {
    let reads = FakeReads {
        toolset: Scripted::Value(BTreeMap::from([
            (String::from("srv-a"), grants_of(&["read", "write"])),
            (String::from("srv-b"), grants_of(&["list"])),
        ])),
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1", "ts-2"]);
    let outcome = toolset_tool_permissions(&reads, Some(&permission), true)
        .await
        .unwrap();
    assert_eq!(outcome.get("srv-a"), Some(&grants_of(&["read", "write"])));
    assert_eq!(outcome.get("srv-b"), Some(&grants_of(&["list"])));
    assert_eq!(
        reads.calls(),
        expected(&["resolve_toolset_grants:[\"ts-1\", \"ts-2\"]:true"])
    );
}

#[tokio::test]
async fn toolset_grants_are_unloadable_when_the_resolver_returns_nothing() {
    let reads = FakeReads::standard();
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tool_permissions(&reads, Some(&permission), false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
    assert!(error.to_string().contains("ts-1"));
    assert_eq!(
        reads.calls(),
        expected(&["resolve_toolset_grants:[\"ts-1\"]:false"])
    );
}

#[tokio::test]
async fn toolset_grants_propagate_a_resolver_fault() {
    let reads = FakeReads {
        toolset: Scripted::Fault,
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tool_permissions(&reads, Some(&permission), false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Read { .. }));
}

#[tokio::test]
async fn toolset_tools_for_a_server_select_its_grants() {
    let reads = FakeReads {
        toolset: Scripted::Value(BTreeMap::from([
            (String::from("srv-a"), grants_of(&["read"])),
            (String::from("srv-b"), Vec::new()),
        ])),
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1"]);
    assert_eq!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-a", false)
            .await
            .unwrap(),
        Some(grants_of(&["read"]))
    );
    assert_eq!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-b", false)
            .await
            .unwrap(),
        Some(Vec::new())
    );
    assert!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-c", false)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn toolset_tools_are_none_or_unloadable_without_grants() {
    let reads = FakeReads::standard();
    assert!(
        toolset_tools_for_server(&reads, None, "srv-a", false)
            .await
            .unwrap()
            .is_none()
    );
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tools_for_server(&reads, Some(&permission), "srv-a", false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
}

#[test]
fn wildcard_constant_is_the_star() {
    assert_eq!(MCP_ALL_TOOLS_WILDCARD, "*");
}

#[test]
fn union_is_none_when_the_direct_grant_holds_the_wildcard() {
    let direct = grants_of(&["*", "read"]);
    let via = grants_of(&["write"]);
    assert!(union_tool_grants(Some(&direct), Some(&via)).is_none());
}

#[test]
fn union_is_none_when_neither_source_restricts() {
    assert!(union_tool_grants(None, None).is_none());
}

#[test]
fn union_keeps_a_single_source_and_merges_both() {
    let direct = grants_of(&["b", "a"]);
    let via = grants_of(&["a", "c"]);
    assert_eq!(
        union_tool_grants(Some(&direct), None),
        Some(grants_of(&["a", "b"]))
    );
    assert_eq!(
        union_tool_grants(None, Some(&via)),
        Some(grants_of(&["a", "c"]))
    );
    assert_eq!(
        union_tool_grants(Some(&direct), Some(&via)),
        Some(grants_of(&["a", "b", "c"]))
    );
}

#[test]
fn union_keeps_an_empty_direct_grant_as_a_restriction() {
    let empty: Vec<String> = Vec::new();
    assert_eq!(union_tool_grants(Some(&empty), None), Some(Vec::new()));
    assert_eq!(union_tool_grants(None, Some(&empty)), Some(Vec::new()));
}

#[test]
fn union_ignores_the_wildcard_in_a_toolset_grant() {
    let via = grants_of(&["*"]);
    assert_eq!(union_tool_grants(None, Some(&via)), Some(grants_of(&["*"])));
}

#[test]
fn union_matches_the_wildcard_exactly() {
    let doubled = grants_of(&["**"]);
    assert_eq!(
        union_tool_grants(Some(&doubled), None),
        Some(grants_of(&["**"]))
    );
}

#[tokio::test]
async fn nothing_declares_toolsets_without_an_auth() {
    let reads = FakeReads::standard();
    assert!(!key_or_team_declares_toolsets(&reads, None).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn a_key_relation_with_toolsets_declares_them() {
    let reads = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission: Some(row_with_toolsets("op", &["ts-1"])),
        ..mcp_auth()
    };
    assert!(key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn a_key_without_toolsets_and_without_a_team_declares_nothing() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        object_permission: Some(row("op")),
        ..mcp_auth()
    };
    assert!(!key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn an_unhydrated_key_is_loaded_before_the_toolset_check() {
    let reads = FakeReads {
        permission: Scripted::Value(Some(row_with_toolsets("op-key", &["ts-1"]))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..mcp_auth()
    };
    assert!(key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "permission:op-key:false"])
    );
}

#[tokio::test]
async fn a_team_declares_its_toolsets() {
    let auth = McpAuth {
        team_id: Some(String::from("t1")),
        ..mcp_auth()
    };
    let declaring = FakeReads {
        team: Scripted::Value(Some(row_with_toolsets("op-team", &["ts-1"]))),
        ..FakeReads::standard()
    };
    assert!(key_or_team_declares_toolsets(&declaring, Some(&auth)).await);
    assert_eq!(
        declaring.calls(),
        expected(&["has_database", "team_permission:t1:false"])
    );

    let outlined = FakeReads {
        team: Scripted::Value(Some(row("op-team"))),
        ..FakeReads::standard()
    };
    assert!(!key_or_team_declares_toolsets(&outlined, Some(&auth)).await);

    let absent = FakeReads::standard();
    assert!(!key_or_team_declares_toolsets(&absent, Some(&auth)).await);

    let dashboard = FakeReads::standard();
    let ui_auth = McpAuth {
        team_id: Some(String::from("litellm-dashboard")),
        ..mcp_auth()
    };
    assert!(!key_or_team_declares_toolsets(&dashboard, Some(&ui_auth)).await);
    assert_eq!(dashboard.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn faults_declare_nothing() {
    let key_fault = FakeReads {
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let key_record = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..mcp_auth()
    };
    assert!(!key_or_team_declares_toolsets(&key_fault, Some(&key_record)).await);

    let team_fault = FakeReads {
        team: Scripted::Fault,
        ..FakeReads::standard()
    };
    let team_record = McpAuth {
        team_id: Some(String::from("t1")),
        ..mcp_auth()
    };
    assert!(!key_or_team_declares_toolsets(&team_fault, Some(&team_record)).await);
    assert_eq!(
        team_fault.calls(),
        expected(&["has_database", "team_permission:t1:false"])
    );
}
