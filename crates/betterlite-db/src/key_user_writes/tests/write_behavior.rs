#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deadpool_postgres::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::set_default;

use super::postgres::{row_count, service, user_column_text};
use super::support::{
    BrokenSink, Harness, Ports, SinkBehaviour, alias_update, capture_logs, failing_key_write,
    fixed_now, key_column, key_write, user_column, user_write,
};
use crate::budget_permission_rows::Timestamp;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::{UserColumn, UserColumnOp, UserUpdateOps, UserWrite};
use crate::key_user_writes::hash::hash_token;
use crate::key_user_writes::retry::{
    CallType, FAILURE_REPORT_DEADLINE, FailureEvent, report_failure,
};
use crate::key_user_writes::{KeyUserWriteError, KeyUserWrites, UserUpdate, WritePorts};

#[tokio::test]
async fn key_null_list_columns_decode_as_empty() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let token = hash_token("sk-null-lists");
    client
        .execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\") VALUES ($1, $2)",
            &[&token, &"stored"],
        )
        .await
        .unwrap();
    let inserted = writes
        .insert_key(&SecretString::from("sk-null-lists"), &key_write("ignored"))
        .await
        .unwrap();
    assert!(inserted.models.is_empty());
    assert_eq!(inserted.key_alias.as_deref(), Some("stored"));
    let updated = writes
        .update_key(
            &SecretString::from("sk-null-lists"),
            &alias_update("changed"),
        )
        .await
        .unwrap()
        .data
        .expect("updated row");
    assert!(updated.models.is_empty());
    assert_eq!(updated.key_alias.as_deref(), Some("changed"));
    harness.close().await;
}

#[tokio::test]
async fn fresh_user_models_column_defaults_to_empty_object() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let fresh = writes
        .insert_user(&user_write("user-fresh", "fresh"))
        .await
        .unwrap();
    assert!(fresh.models.is_empty());
    assert_eq!(
        user_column_text(&client, "models", "user-fresh")
            .await
            .as_deref(),
        Some("{}")
    );
    harness.close().await;
}

async fn update_user_stamp(
    writes: &KeyUserWrites<'_, Client>,
    user_id: &str,
    update: Option<UserWrite>,
) -> LiteLLMUserTable {
    writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write(user_id, "from-data"),
            update_fields: update,
            custom_update_fields: None,
        })
        .await
        .unwrap()
        .data
}

fn user_updated_at(stamp: Timestamp) -> UserWrite {
    UserWrite(BTreeMap::from([(
        UserColumn::UpdatedAt,
        TokenValue::Timestamp(stamp),
    )]))
}

async fn assert_stored_updated_at(client: &Client, expected: &str) {
    let stored = user_column_text(client, "updated_at", "user-stamp").await;
    assert_eq!(stored.as_deref(), Some(expected));
}

#[tokio::test]
async fn key_update_stamps_updated_at() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let token = hash_token("sk-stamp");
    client
        .execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\", \"updated_at\") VALUES ($1, $2, '1999-01-01 00:00:00')",
            &[&token, &"stored"],
        )
        .await
        .unwrap();
    let stamped = writes
        .update_key(&SecretString::from("sk-stamp"), &alias_update("stamped"))
        .await
        .unwrap()
        .data
        .expect("updated row");
    assert_eq!(stamped.updated_at.map(Timestamp::utc), Some(fixed_now()));
    let explicit = Timestamp::Aware((fixed_now() + chrono::Duration::hours(1)).fixed_offset());
    let carried = writes
        .update_key(
            &SecretString::from("sk-stamp"),
            &key_column(TokenColumn::UpdatedAt, TokenValue::Timestamp(explicit)),
        )
        .await
        .unwrap()
        .data
        .expect("updated row");
    assert_eq!(
        carried.updated_at.map(Timestamp::utc),
        Some(fixed_now() + chrono::Duration::hours(1))
    );
    harness.close().await;
}

#[tokio::test]
async fn user_update_timestamps_follow_columns() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"updated_at\") VALUES ($1, $2, '1999-01-01 00:00:00')",
            &[&"user-stamp", &"stored"],
        )
        .await
        .unwrap();
    let empty = update_user_stamp(&writes, "user-stamp", Some(UserWrite::default())).await;
    assert_eq!(
        empty.updated_at.map(Timestamp::utc),
        Some(chrono::DateTime::from_timestamp(915_148_800, 0).unwrap())
    );
    assert!(empty.models.is_empty());
    assert!(empty.teams.is_empty());
    assert!(empty.allowed_cache_controls.is_empty());
    assert_stored_updated_at(&client, "1999-01-01 00:00:00").await;
    let stamped = update_user_stamp(&writes, "user-stamp", None).await;
    assert_eq!(stamped.updated_at.map(Timestamp::utc), Some(fixed_now()));
    assert_stored_updated_at(&client, "1970-01-01 00:00:00").await;
    let explicit = Timestamp::Aware((fixed_now() + chrono::Duration::hours(1)).fixed_offset());
    let carried = update_user_stamp(&writes, "user-stamp", Some(user_updated_at(explicit))).await;
    assert_eq!(
        carried.updated_at.map(Timestamp::utc),
        Some(fixed_now() + chrono::Duration::hours(1))
    );
    harness.close().await;
}

async fn apply_user_ops(
    writes: &KeyUserWrites<'_, Client>,
    ops: UserUpdateOps,
) -> LiteLLMUserTable {
    writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-ops", "ops"),
            update_fields: None,
            custom_update_fields: Some(ops),
        })
        .await
        .unwrap()
        .data
}

#[tokio::test]
async fn custom_update_operations_apply_atomically() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let mut seeded = user_write("user-ops", "ops");
    seeded
        .0
        .insert(UserColumn::Spend, TokenValue::Number(Decimal::new(2, 0)));
    seeded
        .0
        .insert(UserColumn::MaxParallelRequests, TokenValue::Integer(1));
    writes.insert_user(&seeded).await.unwrap();
    let increment = UserUpdateOps(BTreeMap::from([(
        UserColumn::Spend,
        UserColumnOp::Increment(TokenValue::Number(Decimal::new(3, 0))),
    )]));
    let incremented = apply_user_ops(&writes, increment).await;
    assert_eq!(incremented.spend, Decimal::new(5, 0));
    let multiply = UserUpdateOps(BTreeMap::from([
        (
            UserColumn::Spend,
            UserColumnOp::Multiply(TokenValue::Number(Decimal::new(2, 0))),
        ),
        (
            UserColumn::MaxParallelRequests,
            UserColumnOp::Increment(TokenValue::Integer(2)),
        ),
    ]));
    let multiplied = apply_user_ops(&writes, multiply).await;
    assert_eq!(multiplied.spend, Decimal::new(10, 0));
    assert_eq!(multiplied.max_parallel_requests, Some(3));
    harness.close().await;
}

async fn update_user_with_new_id(
    writes: &KeyUserWrites<'_, Client>,
    client: &Client,
    old: &str,
    new: &str,
    alias: &str,
) {
    let renamed = writes
        .update_user(&UserUpdate {
            user_id: Some(String::from(old)),
            fields: user_write(new, alias),
            update_fields: None,
            custom_update_fields: None,
        })
        .await
        .unwrap();
    assert_eq!(renamed.user_id, old);
    assert_eq!(renamed.data.user_id, new);
    assert_eq!(
        user_column_text(client, "user_alias", new).await.as_deref(),
        Some(alias)
    );
    assert_eq!(
        row_count(client, "LiteLLM_UserTable", "user_id", old).await,
        0
    );
}

#[tokio::test]
async fn explicit_user_id_renames_and_inserts() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_user(&user_write("user-old", "old"))
        .await
        .unwrap();
    update_user_with_new_id(&writes, &client, "user-old", "user-new", "renamed").await;
    update_user_with_new_id(&writes, &client, "user-absent", "user-created", "created").await;
    harness.close().await;
}

#[tokio::test(start_paused = true)]
async fn stalled_failure_sink_is_bounded_by_the_deadline() {
    let sink = BrokenSink {
        calls: AtomicUsize::new(0),
        behaviour: SinkBehaviour::Stall,
    };
    let error = KeyUserWriteError::MissingField { field: "user_id" };
    let started = tokio::time::Instant::now();
    report_failure(
        &sink,
        FailureEvent {
            call_type: CallType::InsertData,
            error: &error,
            traceback: String::from("traceback"),
            duration: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert!(tokio::time::Instant::now().duration_since(started) >= FAILURE_REPORT_DEADLINE);
}

#[tokio::test]
async fn panicking_failure_sink_keeps_original_error() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let sink = BrokenSink {
        calls: AtomicUsize::new(0),
        behaviour: SinkBehaviour::Panic,
    };
    let writes = KeyUserWrites {
        client: &client,
        ports: WritePorts {
            clock: &ports.clock,
            jitter: &ports.jitter,
            pause: &ports.pause,
            sink: &sink,
        },
    };
    let error = writes
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    assert_eq!(sink.calls.load(Ordering::SeqCst), 3);
    harness.close().await;
}

#[tokio::test]
async fn synchronous_failure_sink_panic_keeps_original_error() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let sink = BrokenSink {
        calls: AtomicUsize::new(0),
        behaviour: SinkBehaviour::PanicBeforeReturning,
    };
    let writes = KeyUserWrites {
        client: &client,
        ports: WritePorts {
            clock: &ports.clock,
            jitter: &ports.jitter,
            pause: &ports.pause,
            sink: &sink,
        },
    };
    let error = writes
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    assert_eq!(sink.calls.load(Ordering::SeqCst), 3);
    harness.close().await;
}

#[tokio::test]
async fn insert_user_requires_user_id() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let error = writes.insert_user(&UserWrite::default()).await.unwrap_err();
    assert!(matches!(
        error,
        KeyUserWriteError::MissingField { field: "user_id" }
    ));
    harness.close().await;
}

#[tokio::test]
async fn insert_payload_token_is_redacted() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    writes
        .insert_key(
            &SecretString::from("sk-secret"),
            &key_column(
                TokenColumn::Token,
                TokenValue::Text(String::from("sk-raw-field")),
            ),
        )
        .await
        .unwrap();
    drop(guard);
    let captured = logs.contents();
    assert!(!captured.contains("sk-raw-field"), "{captured}");
    assert!(captured.contains(&hash_token("sk-secret")), "{captured}");
    harness.close().await;
}

#[tokio::test]
async fn key_update_overrides_data_token_with_lookup() {
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
        .update_key(
            &SecretString::from("sk-abc"),
            &key_column(
                TokenColumn::Token,
                TokenValue::Text(String::from("sk-other")),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.token.expose_secret(), &stored);
    let data = result.data.expect("updated row");
    assert!(
        data.token
            .as_ref()
            .is_some_and(|token| token.expose_secret() == &stored)
    );
    assert_eq!(
        row_count(&client, "LiteLLM_VerificationToken", "token", &stored).await,
        1
    );
    assert_eq!(
        row_count(&client, "LiteLLM_VerificationToken", "token", "sk-other").await,
        0
    );
    harness.close().await;
}

#[tokio::test]
async fn key_update_stamps_settings_updated_at_over_explicit() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    writes
        .insert_key(&SecretString::from("sk-abc"), &key_write("first"))
        .await
        .unwrap();
    let explicit = Timestamp::Aware((fixed_now() - chrono::Duration::hours(1)).fixed_offset());
    let data = writes
        .update_key(
            &SecretString::from("sk-abc"),
            &key_column(
                TokenColumn::SettingsUpdatedAt,
                TokenValue::Timestamp(explicit),
            ),
        )
        .await
        .unwrap()
        .data
        .expect("updated row");
    assert_eq!(
        data.settings_updated_at.map(Timestamp::utc),
        Some(fixed_now())
    );
    harness.close().await;
}

#[tokio::test]
async fn insert_payload_password_is_redacted() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let writes = service(&client, &ports);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    writes
        .insert_user(&user_column(
            "user-secret",
            UserColumn::Password,
            TokenValue::Text(String::from("hunter2")),
        ))
        .await
        .unwrap();
    drop(guard);
    let captured = logs.contents();
    assert!(!captured.contains("hunter2"), "{captured}");
    assert_eq!(
        user_column_text(&client, "password", "user-secret")
            .await
            .as_deref(),
        Some("hunter2")
    );
    harness.close().await;
}
