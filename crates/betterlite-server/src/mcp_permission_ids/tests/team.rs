use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};
use rstest::rstest;

use crate::mcp_permission_ids::decode::McpToolGrantsJsonError;
use crate::mcp_permission_ids::team::{
    ALL_PROXY_MCP_SERVERS, TeamAllowedServersError, TeamObjectPermission,
    resolve_team_allowed_mcp_servers,
};

use super::fixtures::database::TestDatabase;
use super::fixtures::fake_rows::FakeRows;
use super::fixtures::{json, nested_array_grant, registry, registry_server, string_set, strings};

fn tool_permission(raw: JsonData) -> TeamObjectPermission {
    TeamObjectPermission {
        mcp_tool_permissions: Some(raw),
        ..TeamObjectPermission::default()
    }
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
        ..TeamObjectPermission::default()
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
        mcp_access_groups: Some(strings(&["group-a"])),
        ..TeamObjectPermission::default()
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
        mcp_access_groups: Some(strings(&["group-a"])),
        ..TeamObjectPermission::default()
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
    assert_eq!(
        allowed,
        string_set(&[
            "cfg-a",
            "cfg-b",
            "db-1",
            "gone",
            "db-from-group",
            "ghost-tool"
        ])
    );
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

#[rstest]
#[case::null("null")]
#[case::false_flag("false")]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::negative_zero_float("-0.0")]
#[case::empty_string(r#""""#)]
#[case::empty_array("[]")]
#[case::empty_object("{}")]
#[tokio::test]
async fn falsey_tool_grants_are_empty(#[case] raw: &str) {
    let shared = registry(Vec::new(), Vec::new());
    let permission = tool_permission(json(raw));
    let allowed = resolve_team_allowed_mcp_servers::<FakeRows>(&permission, None, &shared, None)
        .await
        .unwrap();
    assert!(allowed.is_empty());
}

#[rstest]
#[case::true_flag("true")]
#[case::integer("7")]
#[case::float("1.5")]
#[case::array(r#"["stale"]"#)]
#[case::string_document(r#""[]""#)]
#[tokio::test]
async fn non_falsey_wrong_shaped_tool_grants_error(#[case] raw: &str) {
    let shared = registry(Vec::new(), Vec::new());
    let permission = tool_permission(json(raw));
    let outcome =
        resolve_team_allowed_mcp_servers::<FakeRows>(&permission, None, &shared, None).await;
    assert!(matches!(
        outcome,
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
