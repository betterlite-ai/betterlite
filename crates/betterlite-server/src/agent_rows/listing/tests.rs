#![cfg(test)]

use betterlite_db::postgres_store::session::PostgresStore;

use super::{AgentListError, get_all_agents_from_db};
use crate::agent_rows::support::Database;

#[tokio::test]
async fn listing_returns_every_agent_newest_first() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers)
               VALUES ('perm-1', ARRAY['server-a']);
               INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params,
                   object_permission_id, created_at, created_by, updated_by)
               VALUES ('older', 'older', '{}'::jsonb, NULL, '2026-01-01 00:00:00', 'creator', 'creator'),
                      ('newest', 'newest', '{}'::jsonb, 'perm-1', '2026-01-03 00:00:00', 'creator', 'creator'),
                      ('middle', 'middle', '{}'::jsonb, NULL, '2026-01-02 00:00:00', 'creator', 'creator');
               INSERT INTO "LiteLLM_AgentIdentity" (agent_id, provider, issuer, tenant_id, client_id, revision)
               VALUES ('newest', 'microsoft_entra', 'https://login.example/v2.0',
                       '550e8400-e29b-41d4-a716-446655440000',
                       '550e8400-e29b-41d4-a716-446655440001', 'rev-1');"#,
        )
        .await;

    let agents = get_all_agents_from_db(&database.store())
        .await
        .expect("the agents list");
    let agent_ids: Vec<&str> = agents.iter().map(|agent| agent.agent_id.as_str()).collect();
    assert_eq!(agent_ids, vec!["newest", "middle", "older"]);

    let newest = agents.first().expect("the newest agent is listed first");
    assert!(newest.identity.is_some());
    assert_eq!(
        newest
            .object_permission
            .as_ref()
            .and_then(|permission| permission.mcp_servers.as_ref()),
        Some(&Some(vec![String::from("server-a")]))
    );
    let middle = agents.get(1).expect("the middle agent is listed");
    assert!(middle.identity.is_none());
    assert!(middle.object_permission.is_none());
    database.close().await;
}

#[tokio::test]
async fn listing_wraps_a_database_failure() {
    let store = PostgresStore { pool: None };
    let error = get_all_agents_from_db(&store)
        .await
        .expect_err("a store without a pool cannot list");
    assert_eq!(
        error.to_string(),
        "Error getting agents from DB: No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
    assert!(matches!(error, AgentListError::Database { .. }));
}

#[tokio::test]
async fn listing_wraps_a_decode_failure() {
    let database = Database::start().await;
    database
        .run(
            r#"
            INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'creator', 'creator');
            INSERT INTO "LiteLLM_AgentIdentity"
                (agent_id, provider, issuer, tenant_id, client_id, revision)
            VALUES
                ('agent-1', 'microsoft_entra', 'https://login.example/v2.0',
                 'sentinel-tenant', '550e8400-e29b-41d4-a716-446655440001', 'rev-1');
            "#,
        )
        .await;

    let error = get_all_agents_from_db(&database.store())
        .await
        .expect_err("the stored tenant is not a UUID");
    assert_eq!(
        error.to_string(),
        "Error getting agents from DB: the agent identity row is unreadable"
    );
    assert!(matches!(error, AgentListError::Decode { .. }));
    database.close().await;
}
