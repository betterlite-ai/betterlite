#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_openai::batches::records::{BatchObject, BatchStatus};
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::budget_permission_rows::JsonData;

use super::batch::{BatchUnchanged, BatchWrite, ManagedBatchRepository};
use super::decoder::decode_managed_batch_record;
use super::file_content::ManagedFileContentRepository;
use super::repository::{PrismaTableRepository, TableError};
use super::session::{AutoRouterSessionRepository, AutoRouterSessionTable};
use super::user_banner::{BannerActions, USER_BANNER_ROW_ID, UserBannerRepository};

mod binding_cases;
mod decoder_cases;
mod support;
use support::{
    Case, Journal, RecordingActions, assert_bindings, captured_logs, dead_redis,
    drop_batch_unique_index, failing_publisher, insert_batch, insert_session, insert_ui_settings,
    observing_publisher, recording_publisher, set_batch_status,
};

const BATCH_JSON: &str = r#"{"id":"batch_1","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"file_1","object":"batch","status":"validating"}"#;

fn batch() -> BatchObject {
    decode_managed_batch_record::<BatchObject>(BATCH_JSON.as_bytes())
        .unwrap()
        .unwrap()
}

fn write_of(record: &BatchObject, status: &str) -> BatchWrite {
    BatchWrite {
        id: record.id.clone(),
        status: status.to_owned(),
        file_object: serde_json::to_string(record).unwrap(),
    }
}

fn unchanged(status: Option<&str>) -> BatchUnchanged {
    BatchUnchanged {
        unified_object_id: None,
        status: status.map(str::to_owned),
        updated_before: None,
    }
}

#[test]
fn missing_client_is_lazy_exact_error() {
    let journal = Journal::default();
    let base =
        PrismaTableRepository::<PolicyTable>::new(None, recording_publisher(&journal), PolicyTable);
    let message = "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
    assert_eq!(base.access().err().unwrap().to_string(), message);
    assert_eq!(
        base.table(|_access| RecordingActions)
            .err()
            .unwrap()
            .to_string(),
        message
    );
    assert!(journal.entries().is_empty());
}

#[tokio::test]
async fn default_read_and_explicit_writer() {
    let case = Case::split().await;
    let journal = Journal::default();
    let routed = PrismaTableRepository::<AutoRouterSessionTable>::new(
        Some(case.client()),
        recording_publisher(&journal),
        AutoRouterSessionTable,
    );
    let access = routed.access().unwrap();
    assert!(std::ptr::eq(access.read, &case.reader));
    assert!(std::ptr::eq(access.write, &case.writer));

    let pinned = PrismaTableRepository::<AutoRouterSessionTable>::with_writer(
        Some(case.client()),
        recording_publisher(&journal),
        AutoRouterSessionTable,
    );
    let access = pinned.access().unwrap();
    assert!(std::ptr::eq(access.read, &case.writer));
    assert!(std::ptr::eq(access.write, &case.writer));

    case.availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let access = routed.access().unwrap();
    assert!(std::ptr::eq(access.read, &case.writer));

    insert_session(
        &case.reader,
        "reader-key",
        "session-1",
        "router-1",
        "from-reader",
        Some("2026-01-01 00:00:00"),
    )
    .await;
    case.availability
        .reader_unavailable
        .store(false, Ordering::SeqCst);
    let repository =
        AutoRouterSessionRepository::new(Some(case.client()), recording_publisher(&journal));
    let row = repository
        .find_latest_for_key("reader-key", "session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.last_model, "from-reader");
    assert!(journal.entries().is_empty());
    case.close().await;
}

#[tokio::test]
async fn configured_tables_publish_after_write() {
    let case = Case::new().await;
    let journal = Journal::default();
    let repository = UserBannerRepository::new(
        Some(case.client()),
        observing_publisher(&journal, &case.writer),
    );
    repository.upsert_settings("from-request").await.unwrap();
    assert_eq!(
        journal.entries(),
        ["publish:litellm_uisettings:visible=true"]
    );
    case.close().await;
}

#[tokio::test]
async fn reads_and_unconfigured_tables_do_not_publish() {
    let case = Case::new().await;
    let journal = Journal::default();
    let publisher = recording_publisher(&journal);
    insert_batch(&case.writer, "read-only", Some("completed"), BATCH_JSON).await;
    let batches = ManagedBatchRepository::new(Some(case.client()), Arc::clone(&publisher));
    assert_eq!(
        batches.load_status("read-only").await.unwrap(),
        Some("completed".to_owned())
    );
    let banner = UserBannerRepository::new(Some(case.client()), Arc::clone(&publisher));
    assert!(banner.get_raw_settings().await.unwrap().is_none());

    let team = PrismaTableRepository::<TeamTable>::new(
        Some(case.client()),
        Arc::clone(&publisher),
        TeamTable,
    );
    let table = team.table(|_access| RecordingActions).unwrap();
    assert!(table.sync.is_none());
    assert_eq!(table.create("row").await.unwrap(), 1);

    let settings = PrismaTableRepository::<UISettings>::new(
        Some(case.client()),
        Arc::clone(&publisher),
        UISettings,
    );
    assert!(
        settings
            .table(|_access| RecordingActions)
            .unwrap()
            .sync
            .is_some()
    );
    assert!(journal.entries().is_empty());
    case.close().await;
}

#[tokio::test]
async fn failed_database_write_does_not_publish() {
    let case = Case::new().await;
    let journal = Journal::default();
    let repository = UserBannerRepository::new(Some(case.client()), recording_publisher(&journal));
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute("DROP TABLE \"LiteLLM_UISettings\"")
        .await
        .unwrap();
    drop(client);
    assert!(repository.upsert_settings("payload").await.is_err());
    assert!(journal.entries().is_empty());
    case.close().await;
}

#[tokio::test]
async fn zero_row_write_still_publishes() {
    let case = Case::new().await;
    let journal = Journal::default();
    let base = PrismaTableRepository::<UISettings>::new(
        Some(case.client()),
        recording_publisher(&journal),
        UISettings,
    );
    let table = base.table(|access| BannerActions { access }).unwrap();
    let updated = table
        .update_many("payload".to_owned(), "missing-id".to_owned())
        .await
        .unwrap();
    assert_eq!(updated, 0);
    assert_eq!(journal.entries(), ["publish:litellm_uisettings"]);
    case.close().await;
}

#[tokio::test]
async fn redis_missing_is_noop() {
    let case = Case::new().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        redis_config_change_publisher::<TableError>(None),
    );
    repository.upsert_settings("no-redis").await.unwrap();
    assert_eq!(
        repository.get_raw_settings().await.unwrap().unwrap(),
        JsonData::String("no-redis".to_owned())
    );
    case.close().await;
}

#[tokio::test]
async fn redis_publish_failure_warns_but_write_succeeds() {
    let case = Case::new().await;
    let (logs, _guard) = captured_logs();
    let cache = dead_redis().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        redis_config_change_publisher::<TableError>(Some(Arc::new(cache))),
    );
    repository.upsert_settings("redis-down").await.unwrap();
    assert_eq!(
        repository.get_raw_settings().await.unwrap().unwrap(),
        JsonData::String("redis-down".to_owned())
    );
    assert!(
        logs.text()
            .contains("config sync publish for litellm_uisettings failed: ")
    );
    case.close().await;
}

#[tokio::test]
async fn injected_publish_failure_after_commit() {
    let case = Case::new().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        failing_publisher("injected publish failure"),
    );
    let error = repository.upsert_settings("committed").await.unwrap_err();
    assert!(matches!(error, TableError::Publish { .. }));
    assert_eq!(
        repository.get_raw_settings().await.unwrap().unwrap(),
        JsonData::String("committed".to_owned())
    );
    case.close().await;
}

#[tokio::test]
async fn status_load_ignores_corrupt_blob() {
    let case = Case::new().await;
    insert_batch(&case.writer, "corrupt", Some("completed"), r#""not json""#).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    assert_eq!(
        repository.load_status("corrupt").await.unwrap(),
        Some("completed".to_owned())
    );
    assert!(
        repository
            .load_batch::<BatchObject>("corrupt")
            .await
            .is_err()
    );
    assert_eq!(repository.load_status("missing").await.unwrap(), None);
    assert_eq!(
        repository
            .load_batch::<BatchObject>("missing")
            .await
            .unwrap(),
        None
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_batch_compare_and_set_race() {
    let case = Case::new().await;
    insert_batch(&case.writer, "batch-race", Some("in_progress"), BATCH_JSON).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let stored = repository
        .load_batch::<BatchObject>("batch-race")
        .await
        .unwrap()
        .unwrap();
    set_batch_status(&case.writer, "batch-race", "cancelled").await;
    assert!(
        !repository
            .compare_and_set(
                &write_of(&stored, "validating"),
                &unchanged(Some("in_progress")),
                Some("racer"),
            )
            .await
            .unwrap()
    );
    assert_eq!(
        repository.load_status("batch-race").await.unwrap(),
        Some("cancelled".to_owned())
    );
    assert!(
        repository
            .compare_and_set(
                &write_of(&stored, "validating"),
                &unchanged(Some("cancelled")),
                Some("racer"),
            )
            .await
            .unwrap()
    );
    assert_eq!(
        repository.load_status("batch-race").await.unwrap(),
        Some("validating".to_owned())
    );
    case.close().await;
}

#[tokio::test]
async fn unchanged_predicate_overrides_batch_id() {
    let case = Case::new().await;
    insert_batch(&case.writer, "row-b", Some("in_progress"), BATCH_JSON).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let stored = repository
        .load_batch::<BatchObject>("row-b")
        .await
        .unwrap()
        .unwrap();
    let predicate = BatchUnchanged {
        unified_object_id: Some("row-b".to_owned()),
        status: Some("in_progress".to_owned()),
        updated_before: None,
    };
    assert!(
        repository
            .compare_and_set(&write_of(&stored, "validating"), &predicate, Some("author"),)
            .await
            .unwrap()
    );
    assert_eq!(
        repository.load_status("row-b").await.unwrap(),
        Some("validating".to_owned())
    );
    assert_eq!(repository.load_status(&stored.id).await.unwrap(), None);
    case.close().await;
}

#[tokio::test]
async fn cas_multiple_matches_returns_true() {
    let case = Case::new().await;
    drop_batch_unique_index(&case.writer).await;
    insert_batch(&case.writer, "duplicate", Some("validating"), BATCH_JSON).await;
    insert_batch(&case.writer, "duplicate", Some("validating"), BATCH_JSON).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let mut stored = repository
        .load_batch::<BatchObject>("duplicate")
        .await
        .unwrap()
        .unwrap();
    stored.status = BatchStatus::Completed;
    assert!(
        repository
            .compare_and_set(
                &write_of(&stored, "completed"),
                &unchanged(Some("validating")),
                None,
            )
            .await
            .unwrap()
    );
    let client = case.writer.session().await.unwrap();
    let rows = client
        .query(
            "SELECT count(*) FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'duplicate' AND status = 'completed'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].try_get::<_, i64>(0).unwrap(), 2);
    drop(client);
    case.close().await;
}

#[tokio::test]
async fn cas_only_three_fields_including_null_author() {
    let case = Case::new().await;
    insert_batch(&case.writer, "author-null", Some("in_progress"), BATCH_JSON).await;
    let client = case.writer.session().await.unwrap();
    client
        .execute(
            "UPDATE \"LiteLLM_ManagedObjectTable\" SET created_by = 'creator', team_id = 'team', api_key = 'key', request_tags = '[\"tag\"]'::jsonb WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap();
    let model_object_id: String = client
        .query_one(
            "SELECT model_object_id FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    drop(client);
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let mut stored = repository
        .load_batch::<BatchObject>("author-null")
        .await
        .unwrap()
        .unwrap();
    stored.status = BatchStatus::Completed;
    assert!(
        repository
            .compare_and_set(
                &write_of(&stored, "completed"),
                &unchanged(Some("in_progress")),
                None,
            )
            .await
            .unwrap()
    );
    let client = case.writer.session().await.unwrap();
    let row = client
        .query_one(
            "SELECT file_object::text, status, updated_by, created_by, team_id, api_key, model_object_id, request_tags::text FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap();
    let file_object: String = row.try_get(0).unwrap();
    let written: BatchObject = serde_json::from_str(&file_object).unwrap();
    assert_eq!(written, stored);
    assert_eq!(row.try_get::<_, String>(1).unwrap(), "completed");
    assert_eq!(row.try_get::<_, Option<String>>(2).unwrap(), None);
    assert_eq!(
        row.try_get::<_, Option<String>>(3).unwrap(),
        Some("creator".to_owned())
    );
    assert_eq!(
        row.try_get::<_, Option<String>>(4).unwrap(),
        Some("team".to_owned())
    );
    assert_eq!(
        row.try_get::<_, Option<String>>(5).unwrap(),
        Some("key".to_owned())
    );
    assert_eq!(row.try_get::<_, String>(6).unwrap(), model_object_id);
    assert_eq!(
        row.try_get::<_, Option<String>>(7).unwrap(),
        Some("[\"tag\"]".to_owned())
    );
    drop(client);
    case.close().await;
}

#[tokio::test]
async fn touch_only_author_zero_match_ok() {
    let case = Case::new().await;
    insert_batch(&case.writer, "touched", Some("in_progress"), BATCH_JSON).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    repository.touch("touched", Some("author")).await.unwrap();
    repository.touch("missing", None).await.unwrap();
    let client = case.writer.session().await.unwrap();
    let row = client
        .query_one(
            "SELECT updated_by, created_by, status FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'touched'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        row.try_get::<_, Option<String>>(0).unwrap(),
        Some("author".to_owned())
    );
    assert_eq!(row.try_get::<_, Option<String>>(1).unwrap(), None);
    assert_eq!(
        row.try_get::<_, Option<String>>(2).unwrap(),
        Some("in_progress".to_owned())
    );
    drop(client);
    case.close().await;
}

#[tokio::test]
async fn batch_database_errors_propagate() {
    let case = Case::new().await;
    insert_batch(&case.writer, "gone", Some("in_progress"), BATCH_JSON).await;
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let stored = batch();
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute("DROP TABLE \"LiteLLM_ManagedObjectTable\"")
        .await
        .unwrap();
    drop(client);
    assert!(matches!(
        repository.load_batch::<BatchObject>("gone").await,
        Err(TableError::Database(_))
    ));
    assert!(matches!(
        repository.load_status("gone").await,
        Err(TableError::Database(_))
    ));
    assert!(matches!(
        repository.touch("gone", None).await,
        Err(TableError::Database(_))
    ));
    assert!(matches!(
        repository
            .compare_and_set(&write_of(&stored, "validating"), &unchanged(None), None)
            .await,
        Err(TableError::Database(_))
    ));
    case.close().await;
}

#[tokio::test]
async fn postgres_file_bytes_new_ids_and_empty_content() {
    let case = Case::new().await;
    let repository = ManagedFileContentRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let first = repository.store(b"bytes").await.unwrap();
    let second = repository.store(b"bytes").await.unwrap();
    let empty = repository.store(&[]).await.unwrap();
    assert_ne!(first, second);
    assert_ne!(first, empty);
    assert_eq!(
        repository.load(&first).await.unwrap(),
        Some(b"bytes".to_vec())
    );
    assert_eq!(
        repository.load(&second).await.unwrap(),
        Some(b"bytes".to_vec())
    );
    assert_eq!(repository.load(&empty).await.unwrap(), Some(Vec::new()));
    case.close().await;
}

#[tokio::test]
async fn file_missing_vs_empty() {
    let case = Case::new().await;
    let repository = ManagedFileContentRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    assert_eq!(repository.load("missing").await.unwrap(), None);
    let empty = repository.store(&[]).await.unwrap();
    assert_eq!(repository.load(&empty).await.unwrap(), Some(Vec::new()));
    case.close().await;
}

#[tokio::test]
async fn file_delete_missing_only_is_swallowed() {
    let case = Case::new().await;
    let repository = ManagedFileContentRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    repository.delete("missing").await.unwrap();
    let row_id = repository.store(b"gone").await.unwrap();
    repository.delete(&row_id).await.unwrap();
    repository.delete(&row_id).await.unwrap();
    assert_eq!(repository.load(&row_id).await.unwrap(), None);
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute("DROP TABLE \"LiteLLM_ManagedFileContentTable\"")
        .await
        .unwrap();
    drop(client);
    assert!(matches!(
        repository.delete("missing").await,
        Err(TableError::Database(_))
    ));
    case.close().await;
}

#[tokio::test]
async fn postgres_latest_session_exact_key_partition() {
    let case = Case::new().await;
    insert_session(
        &case.writer,
        "key-a",
        "session-1",
        "router-1",
        "router-one",
        Some("2026-01-01 00:00:00"),
    )
    .await;
    insert_session(
        &case.writer,
        "key-a",
        "session-1",
        "router-2",
        "router-two",
        Some("2026-01-02 00:00:00"),
    )
    .await;
    insert_session(
        &case.writer,
        "key-a",
        "session-2",
        "router-1",
        "other-session",
        Some("2026-01-03 00:00:00"),
    )
    .await;
    insert_session(
        &case.writer,
        "key-b",
        "session-1",
        "router-1",
        "other-key",
        Some("2026-01-04 00:00:00"),
    )
    .await;
    let repository = AutoRouterSessionRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let row = repository
        .find_latest_for_key("key-a", "session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.last_model, "router-two");
    let row = repository
        .find_latest_for_key("key-b", "session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.last_model, "other-key");
    assert_eq!(
        repository
            .find_latest_for_key("key-a", "session-2")
            .await
            .unwrap()
            .unwrap()
            .last_model,
        "other-session"
    );
    assert!(
        repository
            .find_latest_for_key("key-c", "session-1")
            .await
            .unwrap()
            .is_none()
    );
    case.close().await;
}

#[tokio::test]
async fn session_desc_null_and_tie_semantics() {
    let case = Case::new().await;
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute(
            "ALTER TABLE \"LiteLLM_AutoRouterSession\" ALTER COLUMN last_turn_at DROP NOT NULL",
        )
        .await
        .unwrap();
    drop(client);
    insert_session(
        &case.writer,
        "tie-key",
        "session-1",
        "router-1",
        "tied-one",
        Some("2026-01-02 00:00:00"),
    )
    .await;
    insert_session(
        &case.writer,
        "tie-key",
        "session-1",
        "router-2",
        "tied-two",
        Some("2026-01-02 00:00:00"),
    )
    .await;
    let repository = AutoRouterSessionRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let tied = repository
        .find_latest_for_key("tie-key", "session-1")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(tied.last_model.as_str(), "tied-one" | "tied-two"));
    insert_session(
        &case.writer,
        "tie-key",
        "session-1",
        "router-3",
        "null-turn",
        None,
    )
    .await;
    let latest = repository
        .find_latest_for_key("tie-key", "session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.last_model, "null-turn");
    case.close().await;
}

#[tokio::test]
async fn session_missing_or_invalid_row() {
    let case = Case::new().await;
    insert_session(
        &case.writer,
        "invalid-key",
        "session-1",
        "router-1",
        "invalid",
        Some("2026-01-01 00:00:00"),
    )
    .await;
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute(
            "UPDATE \"LiteLLM_AutoRouterSession\" SET tier_turns = '\"oops\"'::jsonb WHERE api_key = 'invalid-key'",
        )
        .await
        .unwrap();
    drop(client);
    let repository = AutoRouterSessionRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    assert!(
        repository
            .find_latest_for_key("missing-key", "session-1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        repository
            .find_latest_for_key("invalid-key", "session-1")
            .await,
        Err(TableError::Row(_))
    ));
    case.close().await;
}

#[tokio::test]
async fn postgres_banner_verbatim_singleton() {
    let case = Case::new().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    repository.upsert_settings("first").await.unwrap();
    repository.upsert_settings("second").await.unwrap();
    insert_ui_settings(&case.writer, "ui_settings", r#"{"theme":"light"}"#).await;
    repository.upsert_settings("third").await.unwrap();
    let client = case.writer.session().await.unwrap();
    let rows = client
        .query(
            "SELECT id, ui_settings::text FROM \"LiteLLM_UISettings\" ORDER BY id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].try_get::<_, String>(0).unwrap(), "ui_settings");
    assert_eq!(
        rows[0].try_get::<_, String>(1).unwrap(),
        r#"{"theme": "light"}"#
    );
    assert_eq!(rows[1].try_get::<_, String>(0).unwrap(), USER_BANNER_ROW_ID);
    let stored: String = rows[1].try_get(1).unwrap();
    assert_eq!(
        serde_json::from_str::<JsonData>(&stored).unwrap(),
        JsonData::String("third".to_owned())
    );
    drop(client);
    case.close().await;
}

#[tokio::test]
async fn banner_absent_and_raw_json_value() {
    let case = Case::new().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    assert!(repository.get_raw_settings().await.unwrap().is_none());
    insert_ui_settings(&case.writer, USER_BANNER_ROW_ID, r#"{"theme":"light"}"#).await;
    let object = repository.get_raw_settings().await.unwrap().unwrap();
    assert!(matches!(object, JsonData::Object(_)));
    repository
        .upsert_settings(r#"{"theme":"dark"}"#)
        .await
        .unwrap();
    assert_eq!(
        repository.get_raw_settings().await.unwrap().unwrap(),
        JsonData::String(r#"{"theme":"dark"}"#.to_owned())
    );
    case.close().await;
}

#[tokio::test]
async fn banner_write_failure_preserves_old_value() {
    let case = Case::new().await;
    let repository = UserBannerRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    repository.upsert_settings("kept").await.unwrap();
    let client = case.writer.session().await.unwrap();
    client
        .batch_execute(
            "ALTER TABLE \"LiteLLM_UISettings\" ADD CONSTRAINT reject_blocked CHECK (ui_settings <> '\"blocked\"'::jsonb)",
        )
        .await
        .unwrap();
    drop(client);
    assert!(repository.upsert_settings("blocked").await.is_err());
    assert_eq!(
        repository.get_raw_settings().await.unwrap().unwrap(),
        JsonData::String("kept".to_owned())
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_table_binding_identities() {
    let case = Case::new().await;
    let client = case.writer.session().await.unwrap();
    let rows = client
        .query(
            "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = current_schema()",
            &[],
        )
        .await
        .unwrap();
    let present: BTreeSet<String> = rows
        .iter()
        .map(|row| row.try_get::<_, String>(0).unwrap())
        .collect();
    assert_bindings(&present);
    drop(client);
    case.close().await;
}
