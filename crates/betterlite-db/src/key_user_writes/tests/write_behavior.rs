#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rust_decimal::Decimal;
use secrecy::SecretString;
use tracing::subscriber::set_default;

use super::postgres::{row_count, service, user_alias, user_column_text};
use super::support::{
    BrokenSink, Harness, Ports, SinkBehaviour, alias_update, capture_logs, failing_key_write,
    fixed_now, key_column, key_write, user_alias_update, user_write,
};
use crate::budget_permission_rows::Timestamp;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::{UserColumn, UserColumnOp, UserUpdateOps, UserWrite};
use crate::key_user_writes::hash::hash_token;
use crate::key_user_writes::{KeyUserWriteError, KeyUserWrites, UserUpdate, WritePorts};

#[tokio::test]
async fn null_list_columns_decode_as_empty() {
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
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"models\", \"teams\", \"allowed_cache_controls\") VALUES ($1, $2, NULL, NULL, NULL)",
            &[&"user-null-lists", &"stored"],
        )
        .await
        .unwrap();
    let inserted_user = writes
        .insert_user(&user_write("user-null-lists", "ignored"))
        .await
        .unwrap();
    assert!(inserted_user.models.is_empty());
    assert!(inserted_user.teams.is_empty());
    assert!(inserted_user.allowed_cache_controls.is_empty());
    assert_eq!(inserted_user.user_alias.as_deref(), Some("stored"));
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
    let updated_user = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-null-lists", "ignored"),
            update_fields: Some(user_alias_update("changed")),
            custom_update_fields: None,
        })
        .await
        .unwrap()
        .data;
    assert!(updated_user.models.is_empty());
    assert!(updated_user.allowed_cache_controls.is_empty());
    assert_eq!(updated_user.user_alias.as_deref(), Some("changed"));
    harness.close().await;
}

#[tokio::test]
async fn update_timestamps_follow_prisma() {
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
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"updated_at\") VALUES ($1, $2, '1999-01-01 00:00:00')",
            &[&"user-stamp", &"stored"],
        )
        .await
        .unwrap();
    let empty = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-stamp", "from-data"),
            update_fields: Some(UserWrite::default()),
            custom_update_fields: None,
        })
        .await
        .unwrap()
        .data;
    assert_eq!(
        empty.updated_at.map(Timestamp::utc),
        Some(chrono::DateTime::from_timestamp(915_148_800, 0).unwrap())
    );
    assert_eq!(
        user_column_text(&client, "updated_at", "user-stamp")
            .await
            .as_deref(),
        Some("1999-01-01 00:00:00")
    );
    let user_stamped = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-stamp", "from-data"),
            update_fields: None,
            custom_update_fields: None,
        })
        .await
        .unwrap()
        .data;
    assert_eq!(
        user_stamped.updated_at.map(Timestamp::utc),
        Some(fixed_now())
    );
    assert_eq!(
        user_column_text(&client, "updated_at", "user-stamp")
            .await
            .as_deref(),
        Some("1970-01-01 00:00:00")
    );
    let user_carried = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-stamp", "from-data"),
            update_fields: Some(UserWrite(BTreeMap::from([(
                UserColumn::UpdatedAt,
                TokenValue::Timestamp(explicit),
            )]))),
            custom_update_fields: None,
        })
        .await
        .unwrap()
        .data;
    assert_eq!(
        user_carried.updated_at.map(Timestamp::utc),
        Some(fixed_now() + chrono::Duration::hours(1))
    );
    harness.close().await;
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
    let incremented = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-ops", "ops"),
            update_fields: None,
            custom_update_fields: Some(UserUpdateOps(BTreeMap::from([(
                UserColumn::Spend,
                UserColumnOp::Increment(TokenValue::Number(Decimal::new(3, 0))),
            )]))),
        })
        .await
        .unwrap()
        .data;
    assert_eq!(incremented.spend, Decimal::new(5, 0));
    let multiplied = writes
        .update_user(&UserUpdate {
            user_id: None,
            fields: user_write("user-ops", "ops"),
            update_fields: None,
            custom_update_fields: Some(UserUpdateOps(BTreeMap::from([
                (
                    UserColumn::Spend,
                    UserColumnOp::Multiply(TokenValue::Number(Decimal::new(2, 0))),
                ),
                (
                    UserColumn::MaxParallelRequests,
                    UserColumnOp::Increment(TokenValue::Integer(2)),
                ),
            ]))),
        })
        .await
        .unwrap()
        .data;
    assert_eq!(multiplied.spend, Decimal::new(10, 0));
    assert_eq!(multiplied.max_parallel_requests, Some(3));
    harness.close().await;
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
    let renamed = writes
        .update_user(&UserUpdate {
            user_id: Some(String::from("user-old")),
            fields: user_write("user-new", "renamed"),
            update_fields: None,
            custom_update_fields: None,
        })
        .await
        .unwrap();
    assert_eq!(renamed.user_id, "user-old");
    assert_eq!(renamed.data.user_id, "user-new");
    assert_eq!(
        user_alias(&client, "user-new").await.as_deref(),
        Some("renamed")
    );
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-old").await,
        0
    );
    let created = writes
        .update_user(&UserUpdate {
            user_id: Some(String::from("user-absent")),
            fields: user_write("user-created", "created"),
            update_fields: None,
            custom_update_fields: None,
        })
        .await
        .unwrap();
    assert_eq!(created.user_id, "user-absent");
    assert_eq!(created.data.user_id, "user-created");
    assert_eq!(
        user_alias(&client, "user-created").await.as_deref(),
        Some("created")
    );
    assert_eq!(
        row_count(&client, "LiteLLM_UserTable", "user_id", "user-absent").await,
        0
    );
    harness.close().await;
}

#[tokio::test]
async fn stalled_failure_sink_keeps_original_error() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let ports = Ports::new(Duration::ZERO);
    let sink = BrokenSink {
        calls: AtomicUsize::new(0),
        behaviour: SinkBehaviour::Stall,
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
    tokio::time::pause();
    let error = writes
        .insert_key(&SecretString::from("sk-retry"), &failing_key_write())
        .await
        .unwrap_err();
    assert!(matches!(error, KeyUserWriteError::Database { .. }));
    assert_eq!(sink.calls.load(Ordering::SeqCst), 3);
    tokio::time::resume();
    harness.close().await;
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
