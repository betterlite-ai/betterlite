use deadpool_postgres::Client;
use tokio_postgres::types::ToSql;

use super::support::{Db, captured_logs};
use crate::tool_records::overrides::list_overrides_for_tool;
use crate::tool_records::queries::{get_tool, list_tools};
use crate::tool_records::records::{ToolDiscoveryUser, ToolInputPolicy, ToolPolicyOverrideRow};

const INSERT_TOOL_SQL: &str = "INSERT INTO \"LiteLLM_ToolTable\" \
     (tool_id, tool_name, key_hash, input_policy, output_policy, call_count, created_at, updated_at) \
     VALUES ($1, $2, $3, $4, 'untrusted', 0, $5::timestamp, $5::timestamp)";
const INSERT_TOKEN_SQL: &str = "INSERT INTO \"LiteLLM_VerificationToken\" \
     (token, key_alias, user_id, object_permission_id, models) \
     VALUES ($1, $2, $3, $4, ARRAY[]::TEXT[])";
const INSERT_USER_SQL: &str = "INSERT INTO \"LiteLLM_UserTable\" \
     (user_id, user_email, user_alias) VALUES ($1, $2, $3)";
const INSERT_TEAM_SQL: &str = "INSERT INTO \"LiteLLM_TeamTable\" \
     (team_id, team_alias, object_permission_id) VALUES ($1, $2, $3)";

struct ToolSeed<'row> {
    id: &'row str,
    name: &'row str,
    key_hash: Option<&'row str>,
    policy: ToolInputPolicy,
    created_at: &'row str,
}

async fn insert_tool(db: &Db, seed: ToolSeed<'_>) {
    let policy = seed.policy.as_str();
    let params: [&(dyn ToSql + Sync); 5] = [
        &seed.id,
        &seed.name,
        &seed.key_hash,
        &policy,
        &seed.created_at,
    ];
    db.execute(INSERT_TOOL_SQL, &params).await;
}

struct TokenSeed<'row> {
    token: &'row str,
    key_alias: Option<&'row str>,
    user_id: Option<&'row str>,
    object_permission_id: Option<&'row str>,
}

async fn insert_token(db: &Db, seed: TokenSeed<'_>) {
    let params: [&(dyn ToSql + Sync); 4] = [
        &seed.token,
        &seed.key_alias,
        &seed.user_id,
        &seed.object_permission_id,
    ];
    db.execute(INSERT_TOKEN_SQL, &params).await;
}

async fn seed_owner(db: &Db) {
    let user_id = "user-1";
    let email = Some("owner@example.com");
    let alias = Some("owner alias");
    let params: [&(dyn ToSql + Sync); 3] = [&user_id, &email, &alias];
    db.execute(INSERT_USER_SQL, &params).await;
}

#[tokio::test]
async fn list_tools_orders_newest_first_and_attaches_the_discovering_owner() {
    let db = Db::start().await;
    seed_owner(&db).await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-shared",
            key_alias: Some("shared"),
            user_id: Some("user-1"),
            object_permission_id: None,
        },
    )
    .await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-unknown",
            key_alias: None,
            user_id: Some("user-missing"),
            object_permission_id: None,
        },
    )
    .await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-ownerless",
            key_alias: None,
            user_id: None,
            object_permission_id: None,
        },
    )
    .await;
    for (id, key_hash, created_at) in [
        ("tool-old", Some("key-shared"), "2026-01-01 00:00:00"),
        ("tool-mid", Some("key-shared"), "2026-01-02 00:00:00"),
        ("tool-new", Some("key-unknown"), "2026-01-03 00:00:00"),
        ("tool-none", None, "2026-01-04 00:00:00"),
        (
            "tool-ownerless",
            Some("key-ownerless"),
            "2026-01-05 00:00:00",
        ),
    ] {
        insert_tool(
            &db,
            ToolSeed {
                id,
                name: id,
                key_hash,
                policy: ToolInputPolicy::Untrusted,
                created_at,
            },
        )
        .await;
    }
    let client = db.client().await;
    let tools = list_tools(Some(&client), None).await;
    let ids: Vec<&str> = tools.iter().map(|tool| tool.tool_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "tool-ownerless",
            "tool-none",
            "tool-new",
            "tool-mid",
            "tool-old"
        ]
    );
    let owner = ToolDiscoveryUser {
        user_id: "user-1".to_owned(),
        user_email: Some("owner@example.com".to_owned()),
        user_alias: Some("owner alias".to_owned()),
    };
    assert_eq!(tools[0].user, None);
    assert_eq!(tools[1].user, None);
    assert_eq!(tools[2].user, None);
    assert_eq!(tools[3].user, Some(owner.clone()));
    assert_eq!(tools[4].user, Some(owner));
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn list_tools_filters_by_input_policy_newest_first() {
    let db = Db::start().await;
    for (id, policy, created_at) in [
        (
            "tool-trusted",
            ToolInputPolicy::Trusted,
            "2026-01-01 00:00:00",
        ),
        (
            "tool-blocked-old",
            ToolInputPolicy::Blocked,
            "2026-01-02 00:00:00",
        ),
        (
            "tool-blocked-new",
            ToolInputPolicy::Blocked,
            "2026-01-03 00:00:00",
        ),
        (
            "tool-untrusted",
            ToolInputPolicy::Untrusted,
            "2026-01-04 00:00:00",
        ),
    ] {
        insert_tool(
            &db,
            ToolSeed {
                id,
                name: id,
                key_hash: None,
                policy,
                created_at,
            },
        )
        .await;
    }
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
    insert_token(
        &db,
        TokenSeed {
            token: "key-1",
            key_alias: None,
            user_id: Some("user-1"),
            object_permission_id: None,
        },
    )
    .await;
    insert_tool(
        &db,
        ToolSeed {
            id: "tool-1",
            name: "tool-1",
            key_hash: Some("key-1"),
            policy: ToolInputPolicy::Untrusted,
            created_at: "2026-01-01 00:00:00",
        },
    )
    .await;
    db.batch(
        "ALTER TABLE \"LiteLLM_VerificationToken\" RENAME TO \"LiteLLM_VerificationToken_hidden\"",
    )
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
    let token = "key-broken";
    let user_id = Some("user-1");
    let params: [&(dyn ToSql + Sync); 2] = [&token, &user_id];
    db.execute(
        "INSERT INTO \"LiteLLM_VerificationToken\" (token, user_id, models, budget_fallbacks) \
         VALUES ($1, $2, ARRAY[]::TEXT[], '5'::JSONB)",
        &params,
    )
    .await;
    insert_tool(
        &db,
        ToolSeed {
            id: "tool-1",
            name: "tool-1",
            key_hash: Some("key-broken"),
            policy: ToolInputPolicy::Untrusted,
            created_at: "2026-01-01 00:00:00",
        },
    )
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
async fn get_tool_returns_the_row_with_its_owner_and_none_for_an_unknown_name() {
    let db = Db::start().await;
    seed_owner(&db).await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-1",
            key_alias: None,
            user_id: Some("user-1"),
            object_permission_id: None,
        },
    )
    .await;
    insert_tool(
        &db,
        ToolSeed {
            id: "tool-1",
            name: "tool-one",
            key_hash: Some("key-1"),
            policy: ToolInputPolicy::Blocked,
            created_at: "2026-01-01 00:00:00",
        },
    )
    .await;
    let client = db.client().await;
    let tool = get_tool(Some(&client), "tool-one").await.unwrap();
    assert_eq!(tool.tool_id, "tool-1");
    assert_eq!(tool.input_policy, ToolInputPolicy::Blocked);
    assert_eq!(
        tool.user,
        Some(ToolDiscoveryUser {
            user_id: "user-1".to_owned(),
            user_email: Some("owner@example.com".to_owned()),
            user_alias: Some("owner alias".to_owned()),
        })
    );
    assert_eq!(get_tool(Some(&client), "tool-missing").await, None);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn overrides_project_keys_and_teams_of_blocking_permissions() {
    let db = Db::start().await;
    let permissions: [&(dyn ToSql + Sync); 2] = [&"perm-blocking", &"perm-other"];
    db.execute(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (object_permission_id, blocked_tools) \
         VALUES ($1, ARRAY['echo']::TEXT[]), ($2, ARRAY['other']::TEXT[])",
        &permissions,
    )
    .await;
    db.execute(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (object_permission_id, blocked_tools) \
         VALUES ('perm-empty', ARRAY['echo']::TEXT[])",
        &[],
    )
    .await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-a",
            key_alias: Some("alias-a"),
            user_id: None,
            object_permission_id: Some("perm-blocking"),
        },
    )
    .await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-b",
            key_alias: None,
            user_id: None,
            object_permission_id: Some("perm-blocking"),
        },
    )
    .await;
    insert_token(
        &db,
        TokenSeed {
            token: "key-c",
            key_alias: None,
            user_id: None,
            object_permission_id: Some("perm-other"),
        },
    )
    .await;
    let team_id = "team-1";
    let team_alias = Some("team alias");
    let permission_id = Some("perm-blocking");
    let team: [&(dyn ToSql + Sync); 3] = [&team_id, &team_alias, &permission_id];
    db.execute(INSERT_TEAM_SQL, &team).await;
    let client = db.client().await;
    let overrides = list_overrides_for_tool(Some(&client), "echo").await;
    assert_eq!(
        overrides,
        [
            ToolPolicyOverrideRow {
                override_id: "perm-blocking".to_owned(),
                tool_name: "echo".to_owned(),
                team_id: None,
                key_hash: Some("key-a".to_owned()),
                input_policy: ToolInputPolicy::Blocked,
                key_alias: Some("alias-a".to_owned()),
                created_at: None,
                updated_at: None,
            },
            ToolPolicyOverrideRow {
                override_id: "perm-blocking".to_owned(),
                tool_name: "echo".to_owned(),
                team_id: None,
                key_hash: Some("key-b".to_owned()),
                input_policy: ToolInputPolicy::Blocked,
                key_alias: None,
                created_at: None,
                updated_at: None,
            },
            ToolPolicyOverrideRow {
                override_id: "perm-blocking".to_owned(),
                tool_name: "echo".to_owned(),
                team_id: Some("team-1".to_owned()),
                key_hash: None,
                input_policy: ToolInputPolicy::Blocked,
                key_alias: Some("team alias".to_owned()),
                created_at: None,
                updated_at: None,
            },
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
