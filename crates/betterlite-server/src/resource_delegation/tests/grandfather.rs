use std::collections::BTreeSet;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_mcp::registry::servers::SharedRegistry;

use crate::mcp_permission_ids::cleanup::NO_MCP_SERVERS;
use crate::mcp_permission_ids::decode::McpToolGrantsJsonError;
use crate::mcp_permission_ids::team::ALL_PROXY_MCP_SERVERS;
use crate::resource_delegation::ObjectPermissionGrants;
use crate::resource_delegation::grandfather::{GrandfatherError, grandfathered_server_ids};
use crate::resource_delegation::server_validation::{KeyServersError, validate_key_servers};

use super::fixtures::database::TestDatabase;
use super::fixtures::fakes::{FakeRows, PortFailure};
use super::fixtures::{
    denied_detail, json, reads, registry, registry_server, server_grants, string_set, strings,
    team_fields, tool_permission_grants,
};

async fn outcome(
    raw: &str,
    database: &TestDatabase,
    shared: &SharedRegistry,
) -> Result<BTreeSet<String>, GrandfatherError> {
    let grants = tool_permission_grants(json(raw));
    grandfathered_server_ids(Some(&grants), Some(database.prisma()), shared).await
}

async fn validate_grant(
    permission: &mut JsonData,
    team: &TeamFields,
    existing: &ObjectPermissionGrants,
    shared: &SharedRegistry,
    client: Option<PrismaClient<'_>>,
) -> Result<(), KeyServersError<PortFailure>> {
    let rows = FakeRows::silent();
    let grant_reads = reads(shared, client, &rows, None);
    validate_key_servers(
        Some(permission),
        Some(team),
        grant_reads,
        false,
        Some(existing),
    )
    .await
}

#[tokio::test]
async fn grandfather_resolved_only() {
    let database = TestDatabase::start().await;
    database
        .seed_server("db-old", Some("db-old-name"), Some("old-alias"))
        .await;
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-old", "name": "cfg-old", "transport": "http"}"#),
            registry_server(
                r#"{"server_id": "cfg-toolset", "name": "cfg-toolset", "transport": "http"}"#,
            ),
            registry_server(
                r#"{"server_id": "cfg-tool", "name": "cfg-tool", "transport": "http"}"#,
            ),
        ],
        Vec::new(),
    );
    let existing = ObjectPermissionGrants {
        mcp_servers: strings(&["old-alias", "gone", "cfg-old"]),
        mcp_access_groups: strings(&["cfg-extra"]),
        mcp_tool_permissions: json(r#"{"cfg-tool": [], "ghost": ["t"]}"#),
        mcp_toolsets: strings(&["cfg-toolset"]),
        search_tools: strings(&["cfg-extra"]),
    };
    let resolved = grandfathered_server_ids(Some(&existing), Some(database.prisma()), &shared)
        .await
        .unwrap();
    assert_eq!(resolved, string_set(&["cfg-old", "cfg-tool", "db-old"]));
    database.close().await;
}

#[tokio::test]
async fn grandfather_stored_tool_text_keys() {
    let database = TestDatabase::start().await;
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-tool", "name": "cfg-tool", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let grants = tool_permission_grants(json(r#""{\"cfg-tool\": []}""#));
    let resolved = grandfathered_server_ids(Some(&grants), Some(database.prisma()), &shared)
        .await
        .unwrap();
    assert_eq!(resolved, string_set(&["cfg-tool"]));
    database.close().await;
}

#[tokio::test]
async fn grandfather_sentinels_removed() {
    let database = TestDatabase::start().await;
    let shared = registry(
        vec![
            registry_server(
                r#"{"server_id": "sentinel-holder", "name": "all-proxy-mcpservers", "transport": "http"}"#,
            ),
            registry_server(
                r#"{"server_id": "no-holder", "name": "no-mcp-servers", "transport": "http"}"#,
            ),
        ],
        Vec::new(),
    );
    let existing = server_grants(
        Some(&[ALL_PROXY_MCP_SERVERS, NO_MCP_SERVERS, "cfg-old"]),
        None,
        None,
    );
    let resolved = grandfathered_server_ids(Some(&existing), Some(database.prisma()), &shared)
        .await
        .unwrap();
    assert!(resolved.is_empty());
    database.close().await;
}

#[tokio::test]
async fn grandfather_needs_existing_and_client() {
    let database = TestDatabase::start().await;
    let shared = registry(Vec::new(), Vec::new());
    let existing = server_grants(Some(&["cfg-old"]), None, None);
    let without_existing = grandfathered_server_ids(None, Some(database.prisma()), &shared)
        .await
        .unwrap();
    assert!(without_existing.is_empty());
    let without_client = grandfathered_server_ids(Some(&existing), None, &shared)
        .await
        .unwrap();
    assert!(without_client.is_empty());
    database.close().await;
}

#[tokio::test]
async fn grandfather_shrinking_grants_pass_and_widening_denied() {
    let database = TestDatabase::start().await;
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-old", "name": "cfg-old", "transport": "http"}"#),
            registry_server(
                r#"{"server_id": "cfg-extra", "name": "cfg-extra", "transport": "http"}"#,
            ),
            registry_server(
                r#"{"server_id": "cfg-tool", "name": "cfg-tool", "transport": "http"}"#,
            ),
        ],
        Vec::new(),
    );
    let kept = server_grants(Some(&["cfg-old", "cfg-tool"]), None, None);
    let team = team_fields(
        "team-1",
        &[],
        Some(server_grants(Some(&["cfg-old"]), None, None)),
    );
    let db = Some(database.prisma());
    let mut shrunk =
        json(r#"{"mcp_servers": ["cfg-old"], "mcp_tool_permissions": {"cfg-tool": ["t"]}}"#);
    validate_grant(&mut shrunk, &team, &kept, &shared, db)
        .await
        .unwrap();
    let mut widened = json(r#"{"mcp_servers": ["cfg-old", "cfg-extra"]}"#);
    let error = validate_grant(&mut widened, &team, &kept, &shared, db)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP servers not allowed by team 'team-1': ['cfg-extra']. Team allows: ['cfg-old']. Global (allow_all_keys) servers: []."
    );
    database.close().await;
}

#[tokio::test]
async fn grandfather_decode_errors() {
    let database = TestDatabase::start().await;
    let shared = registry(Vec::new(), Vec::new());
    assert!(matches!(
        outcome(r#""{""#, &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::Bounded(_)
        ))
    ));
    assert!(matches!(
        outcome(r#""[]""#, &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        outcome("true", &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        outcome("7", &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        outcome(r#"["cfg-x"]"#, &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    database.close().await;
}

#[tokio::test]
async fn grandfather_null_and_empty_tool_grants_grant_nothing() {
    let database = TestDatabase::start().await;
    let shared = registry(Vec::new(), Vec::new());
    assert!(
        outcome("null", &database, &shared)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(outcome("{}", &database, &shared).await.unwrap().is_empty());
    database.close().await;
}

#[tokio::test]
async fn grandfather_untyped_tool_grants_are_rejected() {
    let database = TestDatabase::start().await;
    let shared = registry(Vec::new(), Vec::new());
    assert!(matches!(
        outcome("false", &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        outcome("0", &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    assert!(matches!(
        outcome(r#""""#, &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::Bounded(_)
        ))
    ));
    assert!(matches!(
        outcome("[]", &database, &shared).await,
        Err(GrandfatherError::ToolGrants(
            McpToolGrantsJsonError::NotAnObject
        ))
    ));
    database.close().await;
}
