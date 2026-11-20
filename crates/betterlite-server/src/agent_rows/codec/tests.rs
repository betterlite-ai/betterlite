#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::identity::{ExecutionMode, IdentityProvider};
use betterlite_core::agents::kill_switch::AgentKillSwitchAuth;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use uuid::Uuid;

use crate::agent_rows::support::Database;

fn words(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

const TENANT_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
const CLIENT_ID: &str = "550e8400-e29b-41d4-a716-446655440001";
const PRINCIPAL_ID: &str = "550e8400-e29b-41d4-a716-446655440002";

const COMPLETE_ROW: &str = r#"
    INSERT INTO "LiteLLM_ObjectPermissionTable"
        (object_permission_id, mcp_servers, mcp_access_groups, mcp_toolsets,
         mcp_tool_permissions, models, agents)
    VALUES
        ('perm-1', ARRAY['server-a'], ARRAY['group-a'], ARRAY['toolset-a'],
         '{"server-a": ["tool-a", "tool-b"]}'::jsonb, ARRAY['gpt-4o'],
         ARRAY['child-agent']);
    INSERT INTO "LiteLLM_AgentsTable"
        (agent_id, agent_name, litellm_params, agent_card_params, static_headers,
         extra_headers, kill_switch, access_group_ids, object_permission_id, spend,
         identity_managed, enabled, execution_mode, tpm_limit, rpm_limit,
         session_tpm_limit, session_rpm_limit, created_at, created_by, updated_at,
         updated_by)
    VALUES
        ('agent-1', 'support', '{"model": "gpt-4o"}'::jsonb, '{"name": "support"}'::jsonb,
         '{"x-team": "support"}'::jsonb, ARRAY['x-extra: one'],
         '{"url": "https://agent.example/stop", "auth": {"type": "bearer", "token": "tok-secret"}}'::jsonb,
         ARRAY['group-a'], 'perm-1', 0.1, true, false, 'delegated', 2000000000, NULL, 10, 20,
         '2026-02-01 10:00:00.123', 'creator', '2026-02-02 11:30:00.456', 'editor');
    INSERT INTO "LiteLLM_AgentIdentity"
        (agent_id, active, provider, issuer, tenant_id, client_id, service_principal_id,
         required_roles, required_scopes, revision, last_authenticated_at)
    VALUES
        ('agent-1', true, 'microsoft_entra', 'https://login.example/v2.0',
         '550e8400-e29b-41d4-a716-446655440000',
         '550e8400-e29b-41d4-a716-446655440001',
         '550e8400-e29b-41d4-a716-446655440002', ARRAY['agent.admin'],
         ARRAY['user_impersonation'], 'rev-1', '2026-01-15 08:00:00');
"#;

#[tokio::test]
async fn decodes_the_scalar_columns_of_a_complete_row() {
    let database = Database::start().await;
    database.run(COMPLETE_ROW).await;

    let agent = database.decode_agent("agent-1").await.agent;

    assert_eq!(agent.agent_id, "agent-1");
    assert_eq!(agent.agent_name, "support");
    assert!(agent.identity_managed);
    assert!(!agent.enabled.0);
    assert_eq!(agent.execution_mode, ExecutionMode::Delegated);
    assert!(!agent.jwt_auth_configured);
    assert!(agent.keys.is_none());
    assert!(agent.search_score.is_none());
    assert_eq!(
        agent.created_at,
        Some(instant("2026-02-01T10:00:00.123+00:00"))
    );
    assert_eq!(
        agent.updated_at,
        Some(instant("2026-02-02T11:30:00.456+00:00"))
    );
    assert_eq!(agent.created_by.as_deref(), Some("creator"));
    assert_eq!(agent.updated_by.as_deref(), Some("editor"));
    assert_eq!(agent.spend, Some(decimal("0.1")));
    assert_eq!(agent.tpm_limit, Some(2_000_000_000));
    assert_eq!(agent.rpm_limit, None);
    assert_eq!(agent.session_tpm_limit, Some(10));
    assert_eq!(agent.session_rpm_limit, Some(20));
    assert_eq!(agent.extra_headers, Some(words(&["x-extra: one"])));
    assert_eq!(agent.access_group_ids, Some(words(&["group-a"])));
}

#[tokio::test]
async fn decodes_the_json_columns_of_a_complete_row() {
    let database = Database::start().await;
    database.run(COMPLETE_ROW).await;

    let agent = database.decode_agent("agent-1").await.agent;

    assert_eq!(
        agent
            .litellm_params
            .as_ref()
            .and_then(|params| params.get("model")),
        Some(&serde_json::json!("gpt-4o"))
    );
    assert_eq!(
        serde_json::to_value(&agent.agent_card_params).unwrap(),
        serde_json::json!({"name": "support"})
    );
    assert_eq!(
        agent.static_headers,
        Some(BTreeMap::from([(
            String::from("x-team"),
            String::from("support")
        )]))
    );
    let switch = agent
        .kill_switch
        .as_ref()
        .expect("the row has a kill switch");
    assert_eq!(
        serde_json::to_value(switch).unwrap()["url"],
        serde_json::json!("https://agent.example/stop")
    );
}

#[tokio::test]
async fn decodes_the_identity_of_a_complete_row() {
    let database = Database::start().await;
    database.run(COMPLETE_ROW).await;

    let stored = database.decode_agent("agent-1").await;
    let identity = stored
        .agent
        .identity
        .as_ref()
        .expect("the row has an identity");
    assert_eq!(identity.agent_id, "agent-1");
    assert!(identity.active.0);
    assert_eq!(identity.provider, IdentityProvider::MicrosoftEntra);
    assert_eq!(identity.tenant_id, Uuid::parse_str(TENANT_ID).unwrap());
    assert_eq!(identity.client_id, Uuid::parse_str(CLIENT_ID).unwrap());
    assert_eq!(
        identity.service_principal_id,
        Some(Uuid::parse_str(PRINCIPAL_ID).unwrap())
    );
    assert_eq!(identity.issuer, "https://login.example/v2.0");
    assert_eq!(identity.required_roles, words(&["agent.admin"]));
    assert_eq!(identity.required_scopes, words(&["user_impersonation"]));
    assert_eq!(identity.revision, "rev-1");
    assert_eq!(
        identity.last_authenticated_at,
        Some(instant("2026-01-15T08:00:00+00:00"))
    );
}

#[tokio::test]
async fn decodes_the_object_permission_of_a_complete_row() {
    let database = Database::start().await;
    database.run(COMPLETE_ROW).await;

    let stored = database.decode_agent("agent-1").await;
    let permission = stored
        .agent
        .object_permission
        .as_ref()
        .expect("the row has an object permission");
    assert_eq!(permission.mcp_servers, Some(Some(words(&["server-a"]))));
    assert_eq!(
        permission.mcp_access_groups,
        Some(Some(words(&["group-a"])))
    );
    assert_eq!(permission.mcp_toolsets, Some(Some(words(&["toolset-a"]))));
    assert_eq!(permission.models, Some(Some(words(&["gpt-4o"]))));
    assert_eq!(permission.agents, Some(Some(words(&["child-agent"]))));
    assert_eq!(
        permission
            .mcp_tool_permissions
            .as_ref()
            .expect("tool permissions are present"),
        &Some(BTreeMap::from([(
            String::from("server-a"),
            words(&["tool-a", "tool-b"])
        )]))
    );
}

#[tokio::test]
async fn keeps_the_raw_columns_the_writer_needs() {
    let database = Database::start().await;
    database.run(COMPLETE_ROW).await;

    let stored = database.decode_agent("agent-1").await;
    assert_eq!(stored.object_permission_id.as_deref(), Some("perm-1"));
    assert_eq!(
        stored
            .litellm_params_column
            .as_ref()
            .and_then(|column| column.get("model")),
        Some(&serde_json::json!("gpt-4o"))
    );
}

#[tokio::test]
async fn decodes_a_row_without_identity_or_object_permission() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'creator', 'creator')"#,
        )
        .await;

    let stored = database.decode_agent("agent-1").await;
    let agent = &stored.agent;

    assert!(agent.identity.is_none());
    assert!(agent.object_permission.is_none());
    assert!(stored.object_permission_id.is_none());
    assert!(agent.litellm_params.is_none());
    assert!(agent.kill_switch.is_none());
    assert!(stored.kill_switch_column.is_none());
    assert!(!agent.identity_managed);
    assert!(agent.enabled.0);
    assert_eq!(agent.execution_mode, ExecutionMode::Autonomous);
    assert_eq!(agent.spend, Some(Decimal::ZERO));
    assert_eq!(agent.static_headers, Some(BTreeMap::new()));
    assert_eq!(agent.extra_headers, Some(Vec::new()));
    assert_eq!(agent.access_group_ids, Some(Vec::new()));
    assert!(agent.created_at.is_some());
    assert!(agent.updated_at.is_some());
}

#[tokio::test]
async fn null_role_and_scope_arrays_decode_as_empty_lists() {
    let database = Database::start().await;
    database
        .run(
            r#"
            INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'creator', 'creator');
            INSERT INTO "LiteLLM_AgentIdentity"
                (agent_id, provider, issuer, tenant_id, client_id, required_roles,
                 required_scopes, revision)
            VALUES
                ('agent-1', 'microsoft_entra', 'https://login.example/v2.0',
                 '550e8400-e29b-41d4-a716-446655440000',
                 '550e8400-e29b-41d4-a716-446655440001', NULL, NULL, 'rev-1');
            "#,
        )
        .await;

    let stored = database.decode_agent("agent-1").await;
    let identity = stored
        .agent
        .identity
        .as_ref()
        .expect("the row has an identity");
    assert!(identity.required_roles.is_empty());
    assert!(identity.required_scopes.is_empty());
    assert!(identity.service_principal_id.is_none());
    assert!(identity.last_authenticated_at.is_none());
    assert!(identity.active.0);
}

#[tokio::test]
async fn decodes_null_header_columns_as_none() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, static_headers, extra_headers,
                 access_group_ids, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, NULL, NULL, NULL, 'creator', 'creator')"#,
        )
        .await;

    let agent = database.decode_agent("agent-1").await.agent;
    assert!(agent.static_headers.is_none());
    assert!(agent.extra_headers.is_none());
    assert!(agent.access_group_ids.is_none());
}

#[tokio::test]
async fn decodes_null_permission_columns_as_some_none() {
    let database = Database::start().await;
    database
        .run(
            r#"
            INSERT INTO "LiteLLM_ObjectPermissionTable"
                (object_permission_id, mcp_servers, mcp_access_groups, mcp_toolsets,
                 mcp_tool_permissions, models, agents)
            VALUES ('perm-1', NULL, NULL, NULL, NULL, NULL, NULL);
            INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, object_permission_id,
                 created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'perm-1', 'creator', 'creator');
            "#,
        )
        .await;

    let stored = database.decode_agent("agent-1").await;
    let permission = stored
        .agent
        .object_permission
        .as_ref()
        .expect("the row has an object permission");
    assert_eq!(permission.mcp_servers, Some(None));
    assert_eq!(permission.mcp_access_groups, Some(None));
    assert_eq!(permission.mcp_toolsets, Some(None));
    assert_eq!(permission.mcp_tool_permissions, Some(None));
    assert_eq!(permission.models, Some(None));
    assert_eq!(permission.agents, Some(None));
}

#[tokio::test]
async fn reads_litellm_params_stored_as_an_object_and_as_a_string() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, litellm_params, created_by, updated_by)
            VALUES
                ('object-row', 'object', '{}'::jsonb, '{"model": "gpt-4o"}'::jsonb,
                 'creator', 'creator'),
                ('string-row', 'string', '{}'::jsonb, to_jsonb('{"model": "gpt-4o"}'::text),
                 'creator', 'creator')"#,
        )
        .await;

    let object = database.decode_agent("object-row").await.agent;
    let string = database.decode_agent("string-row").await.agent;
    assert_eq!(object.litellm_params, string.litellm_params);
    assert_eq!(
        object
            .litellm_params
            .as_ref()
            .and_then(|params| params.get("model")),
        Some(&serde_json::json!("gpt-4o"))
    );
}

#[tokio::test]
async fn reads_agent_card_params_stored_as_an_object_and_as_a_string() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, created_by, updated_by)
            VALUES
                ('object-row', 'object', '{"name": "support"}'::jsonb, 'creator', 'creator'),
                ('string-row', 'string', to_jsonb('{"name": "support"}'::text),
                 'creator', 'creator')"#,
        )
        .await;

    let object = database.decode_agent("object-row").await.agent;
    let string = database.decode_agent("string-row").await.agent;
    assert_eq!(
        serde_json::to_value(&object.agent_card_params).unwrap(),
        serde_json::to_value(&string.agent_card_params).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&object.agent_card_params).unwrap()["name"],
        serde_json::json!("support")
    );
}

#[tokio::test]
async fn reads_static_headers_stored_as_an_object_and_as_a_string() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, static_headers, created_by, updated_by)
            VALUES
                ('object-row', 'object', '{}'::jsonb, '{"x-team": "support"}'::jsonb,
                 'creator', 'creator'),
                ('string-row', 'string', '{}'::jsonb, to_jsonb('{"x-team": "support"}'::text),
                 'creator', 'creator')"#,
        )
        .await;

    let object = database.decode_agent("object-row").await.agent;
    let string = database.decode_agent("string-row").await.agent;
    assert_eq!(object.static_headers, string.static_headers);
    assert_eq!(
        object.static_headers,
        Some(BTreeMap::from([(
            String::from("x-team"),
            String::from("support")
        )]))
    );
}

#[tokio::test]
async fn reads_kill_switch_stored_as_an_object_and_as_a_string() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, kill_switch, created_by, updated_by)
            VALUES
                ('object-row', 'object', '{}'::jsonb,
                 '{"url": "https://agent.example/stop", "auth": {"type": "bearer", "token": "tok-secret"}}'::jsonb,
                 'creator', 'creator'),
                ('string-row', 'string', '{}'::jsonb,
                 to_jsonb('{"url": "https://agent.example/stop", "auth": {"type": "bearer", "token": "tok-secret"}}'::text),
                 'creator', 'creator')"#,
        )
        .await;

    let object = database.decode_agent("object-row").await.agent;
    let string = database.decode_agent("string-row").await.agent;
    assert_eq!(
        serde_json::to_value(&object.kill_switch).unwrap(),
        serde_json::to_value(&string.kill_switch).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&object.kill_switch).unwrap()["url"],
        serde_json::json!("https://agent.example/stop")
    );
    let Some(AgentKillSwitchAuth::Bearer { token }) = object
        .kill_switch
        .as_ref()
        .and_then(|switch| switch.auth.as_ref())
    else {
        panic!("the object row stores a bearer token");
    };
    assert_eq!(token.expose_secret(), "tok-secret");
}

#[tokio::test]
async fn reads_tool_permissions_stored_as_an_object_and_as_a_string() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_tool_permissions)
               VALUES ('perm-object', '{"server-a": ["tool-a"]}'::jsonb),
                      ('perm-string', to_jsonb('{"server-a": ["tool-a"]}'::text));
               INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params,
                   object_permission_id, created_by, updated_by)
               VALUES ('object-row', 'object', '{}'::jsonb, 'perm-object', 'creator', 'creator'),
                      ('string-row', 'string', '{}'::jsonb, 'perm-string', 'creator', 'creator');"#,
        )
        .await;

    let object = database.decode_agent("object-row").await.agent;
    let string = database.decode_agent("string-row").await.agent;
    let object_permission = object
        .object_permission
        .as_ref()
        .expect("the object row has a permission");
    let string_permission = string
        .object_permission
        .as_ref()
        .expect("the string row has a permission");
    assert_eq!(
        object_permission.mcp_tool_permissions,
        string_permission.mcp_tool_permissions
    );
    assert_eq!(
        object_permission.mcp_tool_permissions,
        Some(Some(BTreeMap::from([(
            String::from("server-a"),
            words(&["tool-a"])
        )])))
    );
}

#[tokio::test]
async fn keeps_the_kill_switch_secret_in_the_stored_row_and_the_response() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, kill_switch, created_by, updated_by)
            VALUES
                ('agent-1', 'support', '{}'::jsonb,
                 '{"url": "https://agent.example/stop", "method": "PATCH", "auth": {"type": "api_key", "api_key": "key-secret"}}'::jsonb,
                 'creator', 'creator')"#,
        )
        .await;

    let stored = database.decode_agent("agent-1").await;
    assert_eq!(
        stored
            .kill_switch_column
            .as_ref()
            .and_then(|column| column.pointer("/auth/api_key")),
        Some(&serde_json::json!("key-secret"))
    );
    let switch = stored
        .agent
        .kill_switch
        .as_ref()
        .expect("the response keeps the kill switch");
    let AgentKillSwitchAuth::ApiKey { api_key, .. } =
        switch.auth.as_ref().expect("the auth is present")
    else {
        panic!("the stored auth is an API key");
    };
    assert_eq!(api_key.expose_secret(), "key-secret");
}

#[tokio::test]
async fn decode_error_text_names_no_column_value() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, created_by, updated_by)
            VALUES ('agent-1', 'support', '"sentinel-card"'::jsonb, 'creator', 'creator')"#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored card is not an agent card");
    };
    assert_eq!(
        error.to_string(),
        "the agent_card_params column is not an agent card"
    );
    assert!(!error.to_string().contains("sentinel"));
}

#[tokio::test]
async fn identity_error_text_names_no_column_value() {
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

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored tenant is not a UUID");
    };
    assert_eq!(error.to_string(), "the agent identity row is unreadable");
    assert!(!error.to_string().contains("sentinel"));
}

#[tokio::test]
async fn rejects_an_unknown_execution_mode() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, execution_mode, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'sideways', 'creator', 'creator')"#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored mode is not a known mode");
    };
    assert_eq!(
        error.to_string(),
        "the execution_mode column is not a known mode"
    );
}

#[tokio::test]
async fn rejects_an_unknown_identity_provider() {
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
                ('agent-1', 'google', 'https://login.example/v2.0',
                 '550e8400-e29b-41d4-a716-446655440000',
                 '550e8400-e29b-41d4-a716-446655440001', 'rev-1');
            "#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored provider is not a known provider");
    };
    assert_eq!(error.to_string(), "the agent identity row is unreadable");
}

#[tokio::test]
async fn rejects_a_non_finite_spend() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, spend, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'NaN'::double precision,
                    'creator', 'creator')"#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored spend is not a finite number");
    };
    assert_eq!(error.to_string(), "the agent spend column is unreadable");
}

#[tokio::test]
async fn rejects_static_headers_that_are_not_an_object() {
    let database = Database::start().await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, static_headers, created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, '["x-team"]'::jsonb,
                    'creator', 'creator')"#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored headers are not an object");
    };
    assert_eq!(error.to_string(), "an agent JSON column is unreadable");
}

#[tokio::test]
async fn rejects_tool_permissions_that_are_not_an_object() {
    let database = Database::start().await;
    database
        .run(
            r#"
            INSERT INTO "LiteLLM_ObjectPermissionTable"
                (object_permission_id, mcp_tool_permissions)
            VALUES ('perm-1', '[1, 2]'::jsonb);
            INSERT INTO "LiteLLM_AgentsTable"
                (agent_id, agent_name, agent_card_params, object_permission_id,
                 created_by, updated_by)
            VALUES ('agent-1', 'support', '{}'::jsonb, 'perm-1', 'creator', 'creator');
            "#,
        )
        .await;

    let Err(error) = database.decode("agent-1").await else {
        panic!("the stored tool permissions are not an object");
    };
    assert_eq!(error.to_string(), "an agent JSON column is unreadable");
}
