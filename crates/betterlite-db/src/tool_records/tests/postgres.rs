use deadpool_postgres::Client;

use super::support::{Db, captured_logs};
use crate::tool_records::overrides::list_overrides_for_tool;
use crate::tool_records::queries::{get_tool, list_tools};
use crate::tool_records::records::{ToolDiscoveryUser, ToolInputPolicy, ToolPolicyOverrideRow};

const TOOL_INSERT: &str = "INSERT INTO \"LiteLLM_ToolTable\" \
     (tool_id, tool_name, key_hash, input_policy, created_at) VALUES ";
const TOKEN_INSERT: &str = "INSERT INTO \"LiteLLM_VerificationToken\" \
     (token, key_alias, user_id, object_permission_id, models) VALUES ";
const OWNER_SQL: &str = "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email, user_alias) \
     VALUES ('user-1', 'owner@example.com', 'owner alias')";

fn owner() -> ToolDiscoveryUser {
    ToolDiscoveryUser {
        user_id: "user-1".to_owned(),
        user_email: Some("owner@example.com".to_owned()),
        user_alias: Some("owner alias".to_owned()),
    }
}

fn blocked_row(
    key_hash: Option<&str>,
    key_alias: Option<&str>,
    team_id: Option<&str>,
) -> ToolPolicyOverrideRow {
    ToolPolicyOverrideRow {
        override_id: "perm-blocking".to_owned(),
        tool_name: "echo".to_owned(),
        team_id: team_id.map(str::to_owned),
        key_hash: key_hash.map(str::to_owned),
        input_policy: ToolInputPolicy::Blocked,
        key_alias: key_alias.map(str::to_owned),
        created_at: None,
        updated_at: None,
    }
}

#[tokio::test]
async fn list_tools_orders_newest_first_and_attaches_the_discovering_owner() {
    let db = Db::start().await;
    db.batch(OWNER_SQL).await;
    db.batch(&format!(
        "{TOKEN_INSERT}('key-shared', 'shared', 'user-1', NULL, '{{}}'), \
         ('key-unknown', NULL, 'user-missing', NULL, '{{}}'), \
         ('key-ownerless', NULL, NULL, NULL, '{{}}')"
    ))
    .await;
    db.batch(&format!(
        "{TOOL_INSERT}('tool-old', 'tool-old', 'key-shared', 'untrusted', '2026-01-01'), \
         ('tool-mid', 'tool-mid', 'key-shared', 'untrusted', '2026-01-02'), \
         ('tool-new', 'tool-new', 'key-unknown', 'untrusted', '2026-01-03'), \
         ('tool-none', 'tool-none', NULL, 'untrusted', '2026-01-04'), \
         ('tool-ownerless', 'tool-ownerless', 'key-ownerless', 'untrusted', '2026-01-05')"
    ))
    .await;
    let client = db.client().await;
    let tools = list_tools(Some(&client), None).await;
    let owners: Vec<(&str, Option<ToolDiscoveryUser>)> = tools
        .iter()
        .map(|tool| (tool.tool_id.as_str(), tool.user.clone()))
        .collect();
    assert_eq!(
        owners,
        [
            ("tool-ownerless", None),
            ("tool-none", None),
            ("tool-new", None),
            ("tool-mid", Some(owner())),
            ("tool-old", Some(owner())),
        ]
    );
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn list_tools_filters_by_input_policy_newest_first() {
    let db = Db::start().await;
    db.batch(&format!(
        "{TOOL_INSERT}('tool-trusted', 'tool-trusted', NULL, 'trusted', '2026-01-01'), \
         ('tool-blocked-old', 'tool-blocked-old', NULL, 'blocked', '2026-01-02'), \
         ('tool-blocked-new', 'tool-blocked-new', NULL, 'blocked', '2026-01-03'), \
         ('tool-untrusted', 'tool-untrusted', NULL, 'untrusted', '2026-01-04')"
    ))
    .await;
    let client = db.client().await;
    let blocked = list_tools(Some(&client), Some(ToolInputPolicy::Blocked)).await;
    let ids: Vec<&str> = blocked.iter().map(|tool| tool.tool_id.as_str()).collect();
    assert_eq!(ids, ["tool-blocked-new", "tool-blocked-old"]);
    let trusted = list_tools(Some(&client), Some(ToolInputPolicy::Trusted)).await;
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0].tool_id, "tool-trusted");
    let untrusted = list_tools(Some(&client), Some(ToolInputPolicy::Untrusted)).await;
    assert_eq!(untrusted.len(), 1);
    assert_eq!(untrusted[0].tool_id, "tool-untrusted");
    assert_eq!(list_tools(Some(&client), None).await.len(), 4);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn an_owner_lookup_failure_keeps_the_tools_and_logs() {
    let db = Db::start().await;
    db.batch(&format!(
        "{TOKEN_INSERT}('key-1', NULL, 'user-1', NULL, '{{}}'); \
         {TOOL_INSERT}('tool-1', 'tool-1', 'key-1', 'untrusted', '2026-01-01'); \
         ALTER TABLE \"LiteLLM_VerificationToken\" RENAME TO \"LiteLLM_VerificationToken_hidden\""
    ))
    .await;
    let (logs, guard) = captured_logs();
    let client = db.client().await;
    let tools = list_tools(Some(&client), None).await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_id, "tool-1");
    assert_eq!(tools[0].user, None);
    let tool = get_tool(Some(&client), "tool-1").await;
    assert_eq!(tool.unwrap().user, None);
    let text = logs.text();
    assert!(text.contains("owner lookup error"), "{text}");
    drop(guard);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn a_token_row_that_cannot_be_decoded_reaches_the_outer_catch() {
    let db = Db::start().await;
    db.batch(&format!(
        "INSERT INTO \"LiteLLM_VerificationToken\" (token, user_id, models, budget_fallbacks) \
         VALUES ('key-broken', 'user-1', '{{}}', '5'::JSONB); \
         {TOOL_INSERT}('tool-1', 'tool-1', 'key-broken', 'untrusted', '2026-01-01')"
    ))
    .await;
    let (logs, guard) = captured_logs();
    let client = db.client().await;
    assert!(list_tools(Some(&client), None).await.is_empty());
    assert_eq!(get_tool(Some(&client), "tool-1").await, None);
    let text = logs.text();
    assert!(text.contains("list_tools error"), "{text}");
    assert!(text.contains("get_tool error"), "{text}");
    drop(guard);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn an_empty_key_hash_never_reaches_the_owner_lookup() {
    let db = Db::start().await;
    db.batch(&format!(
        "INSERT INTO \"LiteLLM_VerificationToken\" (token, models, budget_fallbacks) \
         VALUES ('', '{{}}', '5'::JSONB); \
         {TOOL_INSERT}('tool-empty-key', 'tool-empty-key', '', 'untrusted', '2026-01-01')"
    ))
    .await;
    let client = db.client().await;
    let tools = list_tools(Some(&client), None).await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].user, None);
    let tool = get_tool(Some(&client), "tool-empty-key").await.unwrap();
    assert_eq!(tool.key_hash.as_deref(), Some(""));
    assert_eq!(tool.user, None);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn get_tool_returns_the_row_with_its_owner_and_none_for_an_unknown_name() {
    let db = Db::start().await;
    db.batch(OWNER_SQL).await;
    db.batch(&format!(
        "{TOKEN_INSERT}('key-1', NULL, 'user-1', NULL, '{{}}'); \
         {TOOL_INSERT}('tool-1', 'tool-one', 'key-1', 'blocked', '2026-01-01')"
    ))
    .await;
    let client = db.client().await;
    let tool = get_tool(Some(&client), "tool-one").await.unwrap();
    assert_eq!(tool.tool_id, "tool-1");
    assert_eq!(tool.input_policy, ToolInputPolicy::Blocked);
    assert_eq!(tool.user, Some(owner()));
    assert_eq!(get_tool(Some(&client), "tool-missing").await, None);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn overrides_project_keys_and_teams_of_blocking_permissions() {
    let db = Db::start().await;
    db.batch(&format!(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (object_permission_id, blocked_tools) \
         VALUES ('perm-blocking', ARRAY['echo']), ('perm-other', ARRAY['other']), \
         ('perm-empty', ARRAY['echo']); \
         {TOKEN_INSERT}('key-a', 'alias-a', NULL, 'perm-blocking', '{{}}'), \
         ('key-b', NULL, NULL, 'perm-blocking', '{{}}'), \
         ('key-c', NULL, NULL, 'perm-other', '{{}}'); \
         INSERT INTO \"LiteLLM_TeamTable\" (team_id, team_alias, object_permission_id) \
         VALUES ('team-1', 'team alias', 'perm-blocking')"
    ))
    .await;
    let client = db.client().await;
    assert_eq!(
        list_overrides_for_tool(Some(&client), "echo").await,
        [
            blocked_row(Some("key-a"), Some("alias-a"), None),
            blocked_row(Some("key-b"), None, None),
            blocked_row(None, Some("team alias"), Some("team-1")),
        ]
    );
    assert!(
        list_overrides_for_tool(Some(&client), "missing")
            .await
            .is_empty()
    );
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn an_override_read_failure_falls_back_to_an_empty_list() {
    let db = Db::start().await;
    db.batch("ALTER TABLE \"LiteLLM_TeamTable\" RENAME TO \"LiteLLM_TeamTable_hidden\"")
        .await;
    let (logs, guard) = captured_logs();
    let client = db.client().await;
    assert!(
        list_overrides_for_tool(Some(&client), "echo")
            .await
            .is_empty()
    );
    let text = logs.text();
    assert!(text.contains("list_overrides_for_tool error"), "{text}");
    drop(guard);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn reads_without_a_database_are_empty() {
    assert!(list_tools::<Client>(None, None).await.is_empty());
    assert!(
        list_tools::<Client>(None, Some(ToolInputPolicy::Blocked))
            .await
            .is_empty()
    );
    assert_eq!(get_tool::<Client>(None, "echo").await, None);
    assert!(
        list_overrides_for_tool::<Client>(None, "echo")
            .await
            .is_empty()
    );
}
