#![cfg(test)]

use std::time::Duration;

use deadpool_postgres::Client;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::error::SqlState;
use tracing::subscriber::set_default;

use super::support::{
    Harness, Ports, alias_update, capture_logs, failing_key_write, key_column, key_write,
    user_alias_update, user_column, user_write,
};
use crate::budget_permission_rows::{JsonData, PermissionList};
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::{UserColumn, UserWrite};
use crate::key_user_writes::dispatch::{QueryType, TableName, UpdateSelector, UpdateTarget};
use crate::key_user_writes::hash::hash_token;
use crate::key_user_writes::retry::CallType;
use crate::key_user_writes::{KeyUserWriteError, KeyUserWrites, UserUpdate};
use crate::postgres_store::errors::DbError;

fn service<'service>(
    client: &'service Client,
    ports: &'service Ports,
) -> KeyUserWrites<'service, Client> {
    KeyUserWrites {
        client,
        ports: ports.write_ports(),
    }
}

async fn column_text(client: &Client, column: &str, token: &str) -> Option<String> {
    let sql = format!(
        "SELECT \"{column}\"::text FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1"
    );
    let row = client.query_opt(sql.as_str(), &[&token]).await.unwrap()?;
    row.try_get::<_, Option<String>>(0).unwrap()
}

async fn column_is_null(client: &Client, column: &str, token: &str) -> bool {
    let sql = format!(
        "SELECT (\"{column}\" IS NULL) FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1"
    );
    let row = client.query_one(sql.as_str(), &[&token]).await.unwrap();
    row.get(0)
}

async fn token_rows(client: &Client, token: &str) -> i64 {
    let row = client
        .query_one(
            "SELECT count(*) FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1",
            &[&token],
        )
        .await
        .unwrap();
    row.get(0)
}

async fn user_alias(client: &Client, user_id: &str) -> Option<String> {
    let row = client
        .query_opt(
            "SELECT \"user_alias\" FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = $1",
            &[&user_id],
        )
        .await
        .unwrap()?;
    row.try_get::<_, Option<String>>(0).unwrap()
}

async fn user_rows(client: &Client, user_id: &str) -> i64 {
    let row = client
        .query_one(
            "SELECT count(*) FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = $1",
            &[&user_id],
        )
        .await
        .unwrap();
    row.get(0)
}

#[tokio::test]
async fn key_conflict_no_update() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let first = writes
        .insert_key(&key_write("sk-abc", "first"))
        .await
        .unwrap();
    let second = writes
        .insert_key(&key_write("sk-abc", "second"))
        .await
        .unwrap();
    let stored = hash_token("sk-abc");
    assert_eq!(first.key_alias.as_deref(), Some("first"));
    assert_eq!(second.key_alias.as_deref(), Some("first"));
    assert_eq!(first.token.as_ref().unwrap().expose_secret(), &stored);
    assert_eq!(token_rows(&client, &stored).await, 1);
    assert_eq!(
        column_text(&client, "key_alias", &stored).await.as_deref(),
        Some("first")
    );
    assert_eq!(token_rows(&client, "sk-abc").await, 0);
    harness.close().await;
}

#[tokio::test]
async fn key_null_window_omitted() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let null_limits = key_column("sk-null", TokenColumn::BudgetLimits, TokenValue::Null);
    let empty_limits = key_column(
        "sk-empty",
        TokenColumn::BudgetLimits,
        TokenValue::Json(JsonData::Array(Vec::new())),
    );
    writes.insert_key(&null_limits).await.unwrap();
    writes.insert_key(&empty_limits).await.unwrap();
    let null_token = hash_token("sk-null");
    let empty_token = hash_token("sk-empty");
    assert!(column_is_null(&client, "budget_limits", &null_token).await);
    assert!(!column_is_null(&client, "budget_limits", &empty_token).await);
    assert_eq!(
        column_text(&client, "budget_limits", &empty_token)
            .await
            .as_deref(),
        Some("[]")
    );
    harness.close().await;
}

#[tokio::test]
async fn key_json_null_retained() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let json_null = key_column(
        "sk-json-null",
        TokenColumn::BudgetLimits,
        TokenValue::Json(JsonData::Null),
    );
    writes.insert_key(&json_null).await.unwrap();
    let token = hash_token("sk-json-null");
    assert!(!column_is_null(&client, "budget_limits", &token).await);
    assert_eq!(
        column_text(&client, "budget_limits", &token)
            .await
            .as_deref(),
        Some("null")
    );
    harness.close().await;
}

#[tokio::test]
async fn key_budget_relation() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"max_budget\", \"created_by\", \"updated_by\") VALUES ($1, $2, $3, $4)",
            &[&"budget-1", &5.0_f64, &"creator", &"updater"],
        )
        .await
        .unwrap();
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let budget_key = key_column(
        "sk-budget",
        TokenColumn::BudgetId,
        TokenValue::Text(String::from("budget-1")),
    );
    let linked = writes.insert_key(&budget_key).await.unwrap();
    let relation = linked.litellm_budget_table.expect("budget relation");
    assert_eq!(
        relation.0.get("budget_id"),
        Some(&JsonData::String(String::from("budget-1")))
    );
    assert_eq!(
        relation.0.get("created_by"),
        Some(&JsonData::String(String::from("creator")))
    );
    let plain = writes
        .insert_key(&key_write("sk-plain", "plain"))
        .await
        .unwrap();
    assert!(plain.litellm_budget_table.is_none());
    harness.close().await;
}

#[tokio::test]
async fn user_conflict_no_update() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let first = writes
        .insert_user(&user_write("user-a", "first"))
        .await
        .unwrap();
    let second = writes
        .insert_user(&user_write("user-a", "second"))
        .await
        .unwrap();
    assert_eq!(first.user_alias.as_deref(), Some("first"));
    assert_eq!(second.user_alias.as_deref(), Some("first"));
    assert_eq!(user_rows(&client, "user-a").await, 1);
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("first")
    );
    harness.close().await;
}

#[tokio::test]
async fn user_organization_fk_400() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let orphan = user_column(
        "user-orphan",
        UserColumn::OrganizationId,
        TokenValue::Text(String::from("missing-org")),
    );
    let error = writes.insert_user(&orphan).await.unwrap_err();
    assert!(matches!(
        error,
        KeyUserWriteError::OrganizationMissing { .. }
    ));
    assert_eq!(error.http_status(), Some(400));
    assert_eq!(
        error.to_string(),
        "Foreign Key Constraint failed. Organization ID=missing-org does not exist in LiteLLM_OrganizationTable. Create via `/organization/new`."
    );
    assert_eq!(user_rows(&client, "user-orphan").await, 0);
    harness.close().await;
}

#[tokio::test]
async fn user_other_fk_failures_keep_their_type() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"max_budget\", \"created_by\", \"updated_by\") VALUES ($1, $2, $3, $4)",
            &[&"org-budget", &1.0_f64, &"creator", &"updater"],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO \"LiteLLM_OrganizationTable\" (\"organization_id\", \"organization_alias\", \"budget_id\", \"created_by\", \"updated_by\") VALUES ($1, $2, $3, $4, $5)",
            &[&"org-1", &"acme", &"org-budget", &"creator", &"updater"],
        )
        .await
        .unwrap();
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let mut fields = user_column(
        "user-team",
        UserColumn::OrganizationId,
        TokenValue::Text(String::from("org-1")),
    );
    fields.0.insert(
        UserColumn::TeamId,
        TokenValue::Text(String::from("missing-team")),
    );
    let error = writes.insert_user(&fields).await.unwrap_err();
    let KeyUserWriteError::Database {
        source: DbError::SqlState {
            code, constraint, ..
        },
    } = &error
    else {
        panic!("expected a SQLSTATE failure, got {error}");
    };
    assert_eq!(code, &SqlState::FOREIGN_KEY_VIOLATION);
    assert_eq!(constraint.as_deref(), Some("B2362_UserTable_team_id_fkey"));
    assert_eq!(error.http_status(), None);
    assert_eq!(user_rows(&client, "user-team").await, 0);
    harness.close().await;
}

#[tokio::test]
async fn user_other_failures_keep_their_type() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let duplicate = user_column(
        "user-a",
        UserColumn::SsoUserId,
        TokenValue::Text(String::from("sso-1")),
    );
    writes.insert_user(&duplicate).await.unwrap();
    let clash = user_column(
        "user-b",
        UserColumn::SsoUserId,
        TokenValue::Text(String::from("sso-1")),
    );
    let error = writes.insert_user(&clash).await.unwrap_err();
    assert!(matches!(
        error,
        KeyUserWriteError::Database {
            source: DbError::UniqueViolation { .. }
        }
    ));
    assert_eq!(error.http_status(), None);
    harness.close().await;
}

async fn seed_permission(client: &Client) {
    client
        .execute(
            "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"mcp_servers\") VALUES ($1, $2)",
            &[&"perm-1", &vec![String::from("srv-a")]],
        )
        .await
        .unwrap();
}

async fn seed_permission_key(writes: &KeyUserWrites<'_, Client>) {
    let key = key_column(
        "sk-abc",
        TokenColumn::ObjectPermissionId,
        TokenValue::Text(String::from("perm-1")),
    );
    writes.insert_key(&key).await.unwrap();
}

#[tokio::test]
async fn key_update_stamp_permission_and_return() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    seed_permission(&client).await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    seed_permission_key(&writes).await;
    let stored = hash_token("sk-abc");
    let result = writes
        .update_key(&SecretString::from("sk-abc"), &alias_update("second"))
        .await
        .unwrap();
    assert_eq!(result.token.expose_secret(), &stored);
    let data = result.data.expect("updated row");
    assert_eq!(data.key_alias.as_deref(), Some("second"));
    assert!(data.settings_updated_at.is_some());
    assert_eq!(data.object_permission_id.as_deref(), Some("perm-1"));
    let permission = data.object_permission.expect("permission");
    assert_eq!(
        permission.mcp_servers,
        PermissionList(Some(vec![String::from("srv-a")]))
    );
    assert_eq!(
        column_text(&client, "key_alias", &stored).await.as_deref(),
        Some("second")
    );
    assert_eq!(
        column_text(&client, "settings_updated_at", &stored)
            .await
            .as_deref(),
        Some("1970-01-01 00:00:00")
    );
    harness.close().await;
}

#[tokio::test]
async fn empty_token_is_present() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\") VALUES ($1, $2)",
            &[&"", &"raw"],
        )
        .await
        .unwrap();
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let result = writes
        .update_key(&SecretString::from(""), &alias_update("empty-alias"))
        .await
        .unwrap();
    assert_eq!(result.token.expose_secret(), "");
    assert_eq!(
        column_text(&client, "key_alias", "").await.as_deref(),
        Some("empty-alias")
    );
    let selector = UpdateSelector {
        token: Some(""),
        user_id: None,
        table_name: None,
        query_type: QueryType::Update,
    };
    assert_eq!(selector.target(), UpdateTarget::Key);
    harness.close().await;
}

#[tokio::test]
async fn key_update_without_row_returns_token_only() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let result = writes
        .update_key(&SecretString::from("sk-missing"), &alias_update("x"))
        .await
        .unwrap();
    assert_eq!(result.token.expose_secret(), &hash_token("sk-missing"));
    assert!(result.data.is_none());
    harness.close().await;
}

#[tokio::test]
async fn key_update_accepts_stored_hash() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_key(&key_write("sk-abc", "first"))
        .await
        .unwrap();
    let stored = hash_token("sk-abc");
    let result = writes
        .update_key(&SecretString::from(stored.clone()), &alias_update("second"))
        .await
        .unwrap();
    assert_eq!(result.token.expose_secret(), &stored);
    assert_eq!(
        column_text(&client, "key_alias", &stored).await.as_deref(),
        Some("second")
    );
    harness.close().await;
}

#[tokio::test]
async fn key_insert_requires_token() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let error = writes.insert_key(&alias_update("x")).await.unwrap_err();
    assert!(matches!(
        error,
        KeyUserWriteError::MissingField { field: "token" }
    ));
    harness.close().await;
}

#[tokio::test]
async fn user_update_map_precedence() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-a", "first"))
        .await
        .unwrap();
    let explicit = UserUpdate {
        user_id: None,
        fields: user_write("user-a", "from-data"),
        update_fields: Some(user_alias_update("explicit")),
        custom_update_fields: Some(user_alias_update("custom")),
    };
    writes.update_user(&explicit).await.unwrap();
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("explicit")
    );
    let custom = UserUpdate {
        user_id: None,
        fields: user_write("user-a", "from-data"),
        update_fields: None,
        custom_update_fields: Some(user_alias_update("custom")),
    };
    writes.update_user(&custom).await.unwrap();
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("custom")
    );
    let plain = UserUpdate {
        user_id: None,
        fields: user_write("user-a", "from-data"),
        update_fields: None,
        custom_update_fields: None,
    };
    writes.update_user(&plain).await.unwrap();
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("from-data")
    );
    harness.close().await;
}

#[tokio::test]
async fn explicit_empty_update_map() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-a", "first"))
        .await
        .unwrap();
    let request = UserUpdate {
        user_id: None,
        fields: user_write("user-a", "from-data"),
        update_fields: Some(UserWrite::default()),
        custom_update_fields: Some(user_alias_update("custom")),
    };
    let result = writes.update_user(&request).await.unwrap();
    assert_eq!(result.data.user_alias.as_deref(), Some("first"));
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("first")
    );
    harness.close().await;
}

#[tokio::test]
async fn user_id_dispatch_precedence() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-a", "a"))
        .await
        .unwrap();
    writes
        .insert_user(&user_write("user-b", "b"))
        .await
        .unwrap();
    let request = UserUpdate {
        user_id: Some(String::from("user-a")),
        fields: user_write("user-b", "b-created"),
        update_fields: Some(user_alias_update("explicit")),
        custom_update_fields: None,
    };
    let result = writes.update_user(&request).await.unwrap();
    assert_eq!(result.user_id, "user-a");
    assert_eq!(
        user_alias(&client, "user-a").await.as_deref(),
        Some("explicit")
    );
    assert_eq!(user_alias(&client, "user-b").await.as_deref(), Some("b"));
    assert_eq!(user_rows(&client, "user-b").await, 1);
    let selector = UpdateSelector {
        token: None,
        user_id: Some("user-a"),
        table_name: Some(TableName::User),
        query_type: QueryType::UpdateMany,
    };
    assert_eq!(selector.target(), UpdateTarget::User);
    harness.close().await;
}

#[tokio::test]
async fn user_id_from_data_when_absent() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-b", "b"))
        .await
        .unwrap();
    let request = UserUpdate {
        user_id: None,
        fields: user_write("user-b", "from-data"),
        update_fields: None,
        custom_update_fields: None,
    };
    let result = writes.update_user(&request).await.unwrap();
    assert_eq!(result.user_id, "user-b");
    assert_eq!(
        user_alias(&client, "user-b").await.as_deref(),
        Some("from-data")
    );
    harness.close().await;
}

#[tokio::test]
async fn user_update_requires_user_id() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let error = writes
        .update_user(&UserUpdate::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KeyUserWriteError::MissingField { field: "user_id" }
    ));
    assert_eq!(error.http_status(), None);
    harness.close().await;
}

#[tokio::test]
async fn retry_three_or_ten_seconds() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let fast_ports = Ports::new(Duration::ZERO);
    let fast_writes = service(&client, &fast_ports);
    let error = fast_writes
        .insert_key(&failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    assert_eq!(fast_ports.sink.events().len(), 3);
    assert_eq!(
        fast_ports.pause.delays(),
        vec![Duration::from_millis(500), Duration::from_secs(1)]
    );
    assert_eq!(
        fast_ports.timeline(),
        vec![
            "event insert_data",
            "pause 500",
            "event insert_data",
            "pause 1000",
            "event insert_data",
        ]
    );
    let slow_ports = Ports::new(Duration::from_secs(3));
    let slow_writes = service(&client, &slow_ports);
    slow_writes
        .insert_key(&failing_key_write())
        .await
        .unwrap_err();
    assert_eq!(slow_ports.sink.events().len(), 2);
    assert_eq!(slow_ports.pause.delays(), vec![Duration::from_millis(500)]);
    assert_eq!(
        slow_ports.timeline(),
        vec!["event insert_data", "pause 500", "event insert_data"]
    );
    harness.close().await;
}

fn assert_backoff_log(captured: &str) {
    assert!(
        captured.contains("Backing off... this was attempt #1"),
        "{captured}"
    );
    assert!(
        captured.contains("Backing off... this was attempt #2"),
        "{captured}"
    );
    assert!(
        captured.contains("LiteLLM Prisma Client Exception in insert_data: "),
        "{captured}"
    );
}

#[tokio::test]
async fn retry_events_and_jitter() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::from_secs(1));
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let error = writes.insert_key(&failing_key_write()).await.unwrap_err();
    drop(guard);
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, CallType::InsertData);
    assert_eq!(events[0].duration, Duration::from_secs(1));
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception in insert_data: ")
    );
    assert!(
        events[0].traceback.contains("22P02"),
        "{}",
        events[0].traceback
    );
    assert_eq!(
        ports.pause.delays(),
        vec![Duration::from_millis(500), Duration::from_secs(1)]
    );
    assert_eq!(
        ports.timeline(),
        vec![
            "event insert_data",
            "pause 500",
            "event insert_data",
            "pause 1000",
            "event insert_data",
        ]
    );
    assert_backoff_log(&logs.contents());
    harness.close().await;
}

#[tokio::test]
async fn insert_logs_redact_token_and_name_the_table() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    writes
        .insert_key(&key_write("sk-secret", "logged"))
        .await
        .unwrap();
    writes
        .insert_user(&user_write("user-log", "logged"))
        .await
        .unwrap();
    writes
        .update_key(&SecretString::from("sk-secret"), &alias_update("updated"))
        .await
        .unwrap();
    writes
        .update_user(&UserUpdate {
            user_id: Some(String::from("user-log")),
            fields: user_write("user-log", "from-data"),
            update_fields: None,
            custom_update_fields: None,
        })
        .await
        .unwrap();
    drop(guard);
    let captured = logs.contents();
    assert!(!captured.contains("sk-secret"), "{captured}");
    assert!(captured.contains(&hash_token("sk-secret")), "{captured}");
    for message in [
        "Data Inserted into Keys Table",
        "Data Inserted into User Table",
        "DB Token Table update succeeded",
        "DB User Table - update succeeded",
    ] {
        assert!(captured.contains(message), "{captured}");
    }
    harness.close().await;
}

#[tokio::test]
async fn retry_update_events_use_update_heading() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_key(&key_write("sk-seed", "seeded"))
        .await
        .unwrap();
    let error = writes
        .update_key(&SecretString::from("sk-seed"), &failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    for event in &events {
        assert_eq!(event.call_type, CallType::UpdateData);
        assert!(
            event
                .traceback
                .starts_with("LiteLLM Prisma Client Exception - update_data: ")
        );
    }
    assert_eq!(
        column_text(&client, "key_alias", &hash_token("sk-seed"))
            .await
            .as_deref(),
        Some("seeded")
    );
    harness.close().await;
}

#[tokio::test]
async fn earlier_write_survives_next_failure() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-a", "kept"))
        .await
        .unwrap();
    let broken = user_column(
        "user-b",
        UserColumn::MaxBudget,
        TokenValue::Text(String::from("not-a-number")),
    );
    let error = writes.insert_user(&broken).await.unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    assert_eq!(user_alias(&client, "user-a").await.as_deref(), Some("kept"));
    assert_eq!(user_rows(&client, "user-b").await, 0);
    writes
        .insert_user(&user_write("user-c", "after"))
        .await
        .unwrap();
    assert_eq!(
        user_alias(&client, "user-c").await.as_deref(),
        Some("after")
    );
    harness.close().await;
}
