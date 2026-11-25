use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};

use crate::mcp_permission_ids::decode::McpToolGrantsJsonError;
use crate::mcp_permission_ids::team::{
    ALL_PROXY_MCP_SERVERS, TeamAllowedServersError, TeamObjectPermission,
    resolve_team_allowed_mcp_servers,
};

use super::fixtures::database::TestDatabase;
use super::fixtures::fake_rows::{FakeRows, PortFailure};
use super::fixtures::{json, nested_array_grant, registry, registry_server, string_set, strings};

fn tool_permission(raw: JsonData) -> TeamObjectPermission {
    TeamObjectPermission {
        mcp_servers: None,
        mcp_access_groups: None,
        mcp_tool_permissions: Some(raw),
    }
}

async fn tool_grant_outcome(
    raw: JsonData,
) -> Result<BTreeSet<String>, TeamAllowedServersError<PortFailure>> {
    let shared = registry(Vec::new(), Vec::new());
    let permission = tool_permission(raw);
    resolve_team_allowed_mcp_servers::<FakeRows>(&permission, None, &shared, None).await
}

async fn allowed_for_tool_grants(raw: JsonData) -> BTreeSet<String> {
    tool_grant_outcome(raw).await.unwrap()
}

#[tokio::test]
async fn all_proxy_uses_registry_keys() {
    let config = vec![registry_server(
        r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#,
    )];
    let db = vec![registry_server(
        r#"{"server_id": "db-b", "name": "db-b", "transport": "http"}"#,
    )];
    let shared = registry(config, db);
    let permission = TeamObjectPermission {
        mcp_servers: Some(strings(&[ALL_PROXY_MCP_SERVERS])),
        mcp_access_groups: Some(strings(&["group-a"])),
        mcp_tool_permissions: Some(JsonData::String(String::from("{"))),
    };
    let rows = FakeRows::failing();
    let allowed = resolve_team_allowed_mcp_servers(&permission, None, &shared, Some(&rows))
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-a", "db-b"]));
    assert!(rows.calls().is_empty());
}

#[tokio::test]
async fn all_proxy_matches_among_other_entries() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let permission = TeamObjectPermission {
        mcp_servers: Some(strings(&["ignored", ALL_PROXY_MCP_SERVERS])),
        mcp_access_groups: None,
        mcp_tool_permissions: None,
    };
    let allowed = resolve_team_allowed_mcp_servers::<FakeRows>(&permission, None, &shared, None)
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-a"]));
}

#[tokio::test]
async fn group_failure_keeps_config_servers() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-group", "name": "cfg-group", "transport": "http", "access_groups": ["group-a"]}"#,
        )],
        Vec::new(),
    );
    let permission = TeamObjectPermission {
        mcp_servers: None,
        mcp_access_groups: Some(strings(&["group-a"])),
        mcp_tool_permissions: None,
    };
    let rows = FakeRows::failing();
    let allowed = resolve_team_allowed_mcp_servers(&permission, None, &shared, Some(&rows))
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-group"]));
    let calls = rows.calls();
    assert_eq!(calls.len(), 1_usize);
    assert_eq!(calls[0].access_groups, strings(&["group-a"]));
    assert!(!calls[0].use_writer);
}

#[tokio::test]
async fn group_rows_extend_config_servers() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-group", "name": "cfg-group", "transport": "http", "access_groups": ["group-a"]}"#,
        )],
        Vec::new(),
    );
    let permission = TeamObjectPermission {
        mcp_servers: None,
        mcp_access_groups: Some(strings(&["group-a"])),
        mcp_tool_permissions: None,
    };
    let rows = FakeRows::answering(&["db-from-group"]);
    let allowed = resolve_team_allowed_mcp_servers(&permission, None, &shared, Some(&rows))
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-group", "db-from-group"]));
}

#[tokio::test]
async fn team_union_retains_unresolved() {
    let database = TestDatabase::start().await;
    database
        .seed_server("db-1", Some("db-name"), Some("db-alias"))
        .await;
    let config = vec![
        registry_server(
            r#"{"server_id": "cfg-a", "name": "cfg-a", "alias": "team-alias", "transport": "http"}"#,
        ),
        registry_server(
            r#"{"server_id": "cfg-b", "name": "cfg-b", "transport": "http", "access_groups": ["group-a"]}"#,
        ),
    ];
    let shared = registry(config, Vec::new());
    let permission = TeamObjectPermission {
        mcp_servers: Some(strings(&["team-alias", "db-alias", "gone"])),
        mcp_access_groups: Some(strings(&["group-a"])),
        mcp_tool_permissions: Some(JsonData::String(String::from(
            r#"{"cfg-a": ["t1"], "ghost-tool": ["t2"]}"#,
        ))),
    };
    let rows = FakeRows::answering(&["db-from-group"]);
    let allowed = resolve_team_allowed_mcp_servers(
        &permission,
        Some(database.prisma()),
        &shared,
        Some(&rows),
    )
    .await
    .unwrap();
    let expected = string_set(&[
        "cfg-a",
        "cfg-b",
        "db-1",
        "gone",
        "db-from-group",
        "ghost-tool",
    ]);
    assert_eq!(allowed, expected);
    database.close().await;
}

#[tokio::test]
async fn stored_tool_json_errors() {
    let shared = registry(Vec::new(), Vec::new());
    let malformed = tool_permission(JsonData::String(String::from("{")));
    assert!(matches!(
        resolve_team_allowed_mcp_servers::<FakeRows>(&malformed, None, &shared, None).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::Bounded(PrivateJsonError::Decode(_))
        ))
    ));
    let not_object = tool_permission(JsonData::String(String::from("[]")));
    assert!(matches!(
        resolve_team_allowed_mcp_servers::<FakeRows>(&not_object, None, &shared, None).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    let oversized_text = "x".repeat(MAX_JSON_BYTES.saturating_add(1));
    let oversized = tool_permission(JsonData::String(oversized_text));
    assert!(matches!(
        resolve_team_allowed_mcp_servers::<FakeRows>(&oversized, None, &shared, None).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::Bounded(PrivateJsonError::TooLarge(_, MAX_JSON_BYTES))
        ))
    ));
    let too_deep = tool_permission(JsonData::String(nested_array_grant(MAX_JSON_DEPTH)));
    assert!(matches!(
        resolve_team_allowed_mcp_servers::<FakeRows>(&too_deep, None, &shared, None).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::Bounded(PrivateJsonError::TooDeep(MAX_JSON_DEPTH))
        ))
    ));
}

#[tokio::test]
async fn falsey_tool_grants_are_empty() {
    assert!(allowed_for_tool_grants(json("null")).await.is_empty());
    assert!(allowed_for_tool_grants(json("false")).await.is_empty());
    assert!(allowed_for_tool_grants(json("0")).await.is_empty());
    assert!(allowed_for_tool_grants(json("0.0")).await.is_empty());
    assert!(allowed_for_tool_grants(json("-0.0")).await.is_empty());
    assert!(allowed_for_tool_grants(json(r#""""#)).await.is_empty());
    assert!(allowed_for_tool_grants(json("[]")).await.is_empty());
    assert!(allowed_for_tool_grants(json("{}")).await.is_empty());
}

#[tokio::test]
async fn non_falsey_wrong_shaped_tool_grants_error() {
    assert!(matches!(
        tool_grant_outcome(json("true")).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        tool_grant_outcome(json("7")).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        tool_grant_outcome(json("1.5")).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        tool_grant_outcome(json(r#"["stale"]"#)).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        tool_grant_outcome(json(r#""[]""#)).await,
        Err(TeamAllowedServersError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
}

#[tokio::test]
async fn object_tool_grants_use_their_keys() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let permission = tool_permission(json(r#"{"cfg-a": [], "ghost": ["t"]}"#));
    let allowed = resolve_team_allowed_mcp_servers::<FakeRows>(&permission, None, &shared, None)
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-a", "ghost"]));
}

#[tokio::test]
async fn absent_grants_resolve_to_nothing() {
    let shared = registry(Vec::new(), Vec::new());
    let allowed = resolve_team_allowed_mcp_servers::<FakeRows>(
        &TeamObjectPermission::default(),
        None,
        &shared,
        None,
    )
    .await
    .unwrap();
    assert!(allowed.is_empty());
}
