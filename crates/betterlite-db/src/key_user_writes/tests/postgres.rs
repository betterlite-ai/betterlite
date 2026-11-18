#![cfg(test)]

use std::time::Duration;

use deadpool_postgres::Client;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::error::SqlState;
use tracing::subscriber::set_default;

use super::support::{
    Harness, Ports, RecordedFailure, alias_update, capture_logs, failing_key_write, fixed_now,
    key_column, key_write, user_alias_update, user_column, user_write,
};
use crate::budget_permission_rows::{JsonData, PermissionList, Timestamp};
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::{UserColumn, UserUpdateOps, UserWrite};
use crate::key_user_writes::dispatch::{QueryType, TableName, UpdateSelector, UpdateTarget};
use crate::key_user_writes::hash::hash_token;
use crate::key_user_writes::retry::CallType;
use crate::key_user_writes::{KeyUserWriteError, KeyUserWrites, UserUpdate};
use crate::postgres_store::errors::DbError;

pub(super) fn service<'service>(
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

pub(super) async fn row_count(client: &Client, table: &str, key: &str, value: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM \"{table}\" WHERE \"{key}\" = $1");
    let row = client.query_one(sql.as_str(), &[&value]).await.unwrap();
    row.get(0)
}

pub(super) async fn user_column_text(
    client: &Client,
    column: &str,
    user_id: &str,
) -> Option<String> {
    let sql =
        format!("SELECT \"{column}\"::text FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = $1");
    let row = client.query_opt(sql.as_str(), &[&user_id]).await.unwrap()?;
    row.try_get::<_, Option<String>>(0).unwrap()
}

async fn assert_stored_alias(client: &Client, user_id: &str, expected: &str) {
    let stored = user_column_text(client, "user_alias", user_id).await;
    assert_eq!(stored.as_deref(), Some(expected));
}

#[tokio::test]
async fn key_conflict_no_update() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let first = writes
        .insert_key(&SecretString::from("sk-abc"), &key_write("first"))
        .await
        .unwrap();
    let second = writes
        .insert_key(&SecretString::from("sk-abc"), &key_write("second"))
        .await
        .unwrap();
    let stored = hash_token("sk-abc");
    assert_eq!(first.key_alias.as_deref(), Some("first"));
    assert_eq!(second.key_alias.as_deref(), Some("first"));
    assert!(first.models.is_empty());
    assert!(second.models.is_empty());
    assert_eq!(first.token.as_ref().unwrap().expose_secret(), &stored);
    assert_eq!(
        row_count(&client, "LiteLLM_VerificationToken", "token", &stored).await,
        1
    );
    assert_eq!(
        column_text(&client, "key_alias", &stored).await.as_deref(),
        Some("first")
    );
    assert_eq!(
        row_count(&client, "LiteLLM_VerificationToken", "token", "sk-abc").await,
        0
    );
    harness.close().await;
}

#[tokio::test]
async fn key_null_window_omitted() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let null_limits = key_column(TokenColumn::BudgetLimits, TokenValue::Null);
    let empty_limits = key_column(
        TokenColumn::BudgetLimits,
        TokenValue::Json(JsonData::Array(Vec::new())),
    );
    writes
        .insert_key(&SecretString::from("sk-null"), &null_limits)
        .await
        .unwrap();
    writes
        .insert_key(&SecretString::from("sk-empty"), &empty_limits)
        .await
        .unwrap();
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
    let json_null = key_column(TokenColumn::BudgetLimits, TokenValue::Json(JsonData::Null));
    writes
        .insert_key(&SecretString::from("sk-json-null"), &json_null)
        .await
        .unwrap();
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
        TokenColumn::BudgetId,
        TokenValue::Text(String::from("budget-1")),
    );
    let linked = writes
        .insert_key(&SecretString::from("sk-budget"), &budget_key)
        .await
        .unwrap();
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
        .insert_key(&SecretString::from("sk-plain"), &key_write("plain"))
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
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-a").await,
        1
    );
    assert_eq!(
        user_column_text(&client, "user_alias", "user-a")
            .await
            .as_deref(),
        Some("first")
    );
    harness.close().await;
}

fn assert_organization_missing(error: &KeyUserWriteError) {
    let KeyUserWriteError::OrganizationMissing {
        organization_id,
        source,
    } = error
    else {
        panic!("expected the organization translation, got {error}");
    };
    assert_eq!(organization_id, "missing-org");
    let DbError::SqlState {
        code, constraint, ..
    } = source
    else {
        panic!("expected the preserved foreign key failure, got {source}");
    };
    assert_eq!(code.code(), "23503");
    assert_eq!(
        constraint.as_deref(),
        Some("LiteLLM_UserTable_organization_id_fkey")
    );
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
    assert_organization_missing(&error);
    assert_eq!(error.http_status(), Some(400));
    assert_eq!(
        error.to_string(),
        "Foreign Key Constraint failed. Organization ID=missing-org does not exist in LiteLLM_OrganizationTable. Create via `/organization/new`."
    );
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-orphan").await,
        0
    );
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| event.status == Some(400)));
    assert!(events.iter().all(|event| {
        event
            .source
            .as_deref()
            .is_some_and(|text| text.contains("23503"))
    }));
    harness.close().await;
}

async fn user_in_organization(client: &Client) -> UserWrite {
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
    let mut fields = user_column(
        "user-team",
        UserColumn::OrganizationId,
        TokenValue::Text(String::from("org-1")),
    );
    fields.0.insert(
        UserColumn::TeamId,
        TokenValue::Text(String::from("missing-team")),
    );
    fields
}

#[tokio::test]
async fn user_other_fk_failures_keep_their_type() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let fields = user_in_organization(&client).await;
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
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-team").await,
        0
    );
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
        TokenColumn::ObjectPermissionId,
        TokenValue::Text(String::from("perm-1")),
    );
    writes
        .insert_key(&SecretString::from("sk-abc"), &key)
        .await
        .unwrap();
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
    assert_eq!(data.updated_at.map(Timestamp::utc), Some(fixed_now()));
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
    assert_eq!(
        column_text(&client, "updated_at", &stored).await.as_deref(),
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
        .insert_key(&SecretString::from("sk-abc"), &key_write("first"))
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
async fn empty_token_insert_is_hashed() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_key(&SecretString::from(""), &key_write("empty"))
        .await
        .unwrap();
    let stored = hash_token("");
    assert_eq!(
        column_text(&client, "key_alias", &stored).await.as_deref(),
        Some("empty")
    );
    assert_eq!(
        row_count(&client, "LiteLLM_VerificationToken", "token", "").await,
        0
    );
    harness.close().await;
}

#[rstest]
#[case::explicit_beats_custom(
    Some(user_alias_update("explicit")),
    Some(UserUpdateOps::from(&user_alias_update("custom"))),
    "explicit"
)]
#[case::custom_beats_data(
    None,
    Some(UserUpdateOps::from(&user_alias_update("custom"))),
    "custom"
)]
#[case::data_when_absent(None, None, "from-data")]
#[tokio::test]
async fn user_update_map_precedence(
    #[case] update_fields: Option<UserWrite>,
    #[case] custom_update_fields: Option<UserUpdateOps>,
    #[case] expected: &str,
) {
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
        update_fields,
        custom_update_fields,
    };
    writes.update_user(&request).await.unwrap();
    assert_stored_alias(&client, "user-a", expected).await;
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
        custom_update_fields: Some(UserUpdateOps::from(&user_alias_update("custom"))),
    };
    let result = writes.update_user(&request).await.unwrap();
    assert_eq!(result.data.user_alias.as_deref(), Some("first"));
    assert_eq!(
        user_column_text(&client, "user_alias", "user-a")
            .await
            .as_deref(),
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
    assert_stored_alias(&client, "user-a", "explicit").await;
    assert_stored_alias(&client, "user-b", "b").await;
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-b").await,
        1
    );
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
        user_column_text(&client, "user_alias", "user-b")
            .await
            .as_deref(),
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
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
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
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
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

fn assert_first_insert_event(events: &[RecordedFailure], error: &KeyUserWriteError) {
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
    assert_eq!(events[0].message, error.to_string());
    assert_eq!(events[0].status, None);
    assert_eq!(
        events[0].sql_state.as_ref().map(SqlState::code),
        Some("22P02")
    );
    assert!(events[0].source.is_some());
}

#[tokio::test]
async fn retry_events_and_jitter() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::from_secs(1));
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let error = writes
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    let events = ports.sink.events();
    assert_first_insert_event(&events, &error);
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

async fn perform_logged_writes(writes: &KeyUserWrites<'_, Client>) {
    writes
        .insert_key(&SecretString::from("sk-secret"), &key_write("logged"))
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
}

#[tokio::test]
async fn insert_logs_redact_token_and_name_the_table() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    perform_logged_writes(&writes).await;
    drop(guard);
    let captured = logs.contents();
    assert!(!captured.contains("sk-secret"), "{captured}");
    assert!(captured.contains(&hash_token("sk-secret")), "{captured}");
    assert!(
        [
            "Data Inserted into Keys Table",
            "Data Inserted into User Table",
            "DB Token Table update succeeded",
            "DB User Table - update succeeded",
        ]
        .into_iter()
        .all(|message| captured.contains(message)),
        "{captured}"
    );
    harness.close().await;
}

#[tokio::test]
async fn retry_update_events_use_update_heading() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_key(&SecretString::from("sk-seed"), &key_write("seeded"))
        .await
        .unwrap();
    let error = writes
        .update_key(&SecretString::from("sk-seed"), &failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .all(|event| event.call_type == CallType::UpdateData)
    );
    assert!(events.iter().all(|event| {
        event
            .traceback
            .starts_with("LiteLLM Prisma Client Exception - update_data: ")
    }));
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
    assert_eq!(
        user_column_text(&client, "user_alias", "user-a")
            .await
            .as_deref(),
        Some("kept")
    );
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-b").await,
        0
    );
    writes
        .insert_user(&user_write("user-c", "after"))
        .await
        .unwrap();
    assert_eq!(
        user_column_text(&client, "user_alias", "user-c")
            .await
            .as_deref(),
        Some("after")
    );
    harness.close().await;
}
