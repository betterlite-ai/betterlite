#![cfg(test)]

use super::support::{
    FakeDatabase, FakeFailure, FakeReads, PanickingReads, capture_logs, deleted_row, moment,
    repository, tags_metadata, token_row,
};
use crate::daily_activity_contracts::KeyMetadataRow;
use crate::daily_activity_repository::DailyActivityRepository;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

fn keys(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[tokio::test]
async fn empty_keys_skip_database_and_port() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let metadata = repository.key_metadata(&keys(&[]), None).await.unwrap();
    assert!(metadata.is_empty());
    assert_eq!(repository.database.active_call_count(), 0);
    assert!(repository.database.deleted_call_args().is_empty());
    assert_eq!(repository.proxy_reads.call_count(), 0);
}

#[tokio::test]
async fn active_failure_stops_recovery() {
    let database = FakeDatabase::default();
    *database.active.lock().unwrap() = Err(FakeFailure::Unavailable);
    let repository = repository(database, FakeReads::default());
    assert!(
        repository
            .key_metadata(&keys(&["key"]), None)
            .await
            .is_err()
    );
    assert!(repository.database.deleted_call_args().is_empty());
    assert_eq!(repository.proxy_reads.call_count(), 0);
}

#[tokio::test]
async fn archived_lookup_only_missing() {
    let database = FakeDatabase::default();
    *database.active.lock().unwrap() = Ok(vec![token_row("a")]);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["a", "b"]), None)
        .await
        .unwrap();
    assert_eq!(
        repository.database.deleted_call_args(),
        vec![vec!["b".to_owned()]]
    );
}

#[tokio::test]
async fn newest_deleted_record_selected() {
    let database = FakeDatabase::default();
    *database.deleted.lock().unwrap() = Ok(vec![
        deleted_row("key", moment(100), Some("old")),
        deleted_row("key", moment(200), Some("new")),
    ]);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["key"]), None)
        .await
        .unwrap();
    let call = repository.proxy_reads.last_call();
    assert_eq!(
        call.resolved.get("key").unwrap().key_alias.as_deref(),
        Some("new")
    );
    assert!(!call.resolved.get("key").unwrap().key_exists);
}

#[tokio::test]
async fn equal_deletion_time_first_row_wins() {
    let database = FakeDatabase::default();
    *database.deleted.lock().unwrap() = Ok(vec![
        deleted_row("key", moment(100), Some("first")),
        deleted_row("key", moment(100), Some("second")),
    ]);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["key"]), None)
        .await
        .unwrap();
    let call = repository.proxy_reads.last_call();
    assert_eq!(
        call.resolved.get("key").unwrap().key_alias.as_deref(),
        Some("first")
    );
}

#[tokio::test]
async fn deleted_failure_warns_then_recovers() {
    let (logs, subscriber) = capture_logs();
    let guard = tracing::subscriber::set_default(subscriber);
    let database = FakeDatabase::default();
    *database.deleted.lock().unwrap() = Err(FakeFailure::Unavailable);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["key"]), None)
        .await
        .unwrap();
    drop(guard);
    assert!(
        logs.contents()
            .contains("Could not read deleted verification token metadata")
    );
    assert_eq!(repository.proxy_reads.call_count(), 1);
    assert!(repository.proxy_reads.last_call().resolved.is_empty());
}

#[tokio::test]
async fn deleted_cancel_propagates() {
    let mut database = FakeDatabase::default();
    database.deleted_pending = true;
    let repository = repository(database, FakeReads::default());
    let outcome = tokio::time::timeout(
        Duration::from_millis(50),
        repository.key_metadata(&keys(&["key"]), None),
    )
    .await;
    assert!(outcome.is_err());
    assert_eq!(repository.proxy_reads.call_count(), 0);
}

#[tokio::test]
async fn active_metadata_precedes_archive() {
    let database = FakeDatabase::default();
    let mut active = token_row("key");
    active.key_alias = Some("active".to_owned());
    *database.active.lock().unwrap() = Ok(vec![active]);
    *database.deleted.lock().unwrap() = Ok(vec![deleted_row("key", moment(100), Some("deleted"))]);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["key"]), None)
        .await
        .unwrap();
    let call = repository.proxy_reads.last_call();
    let row = call.resolved.get("key").unwrap();
    assert_eq!(row.key_alias.as_deref(), Some("active"));
    assert!(row.key_exists);
}

#[tokio::test]
async fn recovery_receives_full_set_and_window() {
    let database = FakeDatabase::default();
    *database.active.lock().unwrap() = Ok(vec![token_row("a")]);
    let repository = repository(database, FakeReads::default());
    let window = (
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_secs(60),
    );
    let requested = keys(&["a", "b", "c"]);
    repository
        .key_metadata(&requested, Some(window))
        .await
        .unwrap();
    let call = repository.proxy_reads.last_call();
    assert_eq!(call.api_keys, requested);
    assert_eq!(call.window, Some(window));
    assert_eq!(call.resolved.len(), 1);
}

#[tokio::test]
async fn all_resolved_still_calls_recovery() {
    let database = FakeDatabase::default();
    *database.active.lock().unwrap() = Ok(vec![token_row("a"), token_row("b")]);
    let repository = repository(database, FakeReads::default());
    repository
        .key_metadata(&keys(&["a", "b"]), None)
        .await
        .unwrap();
    assert_eq!(repository.proxy_reads.call_count(), 1);
}

#[tokio::test]
async fn recovery_result_returned_unchanged() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let recovered = KeyMetadataRow {
        api_key: "enriched".to_owned(),
        key_alias: Some("alias".to_owned()),
        team_id: None,
        user_id: None,
        user_email: Some("user@example.com".to_owned()),
        key_exists: true,
        tags: vec!["tag".to_owned()],
    };
    repository
        .proxy_reads
        .reply
        .lock()
        .unwrap()
        .insert("enriched".to_owned(), recovered.clone());
    let metadata = repository
        .key_metadata(&keys(&["key"]), None)
        .await
        .unwrap();
    assert_eq!(
        metadata,
        std::collections::BTreeMap::from([("enriched".to_owned(), recovered)])
    );
}

#[tokio::test]
#[should_panic(expected = "the recovery port failed")]
async fn recovery_error_propagates() {
    let repository: DailyActivityRepository<
        FakeDatabase,
        PanickingReads,
        fn() -> chrono::DateTime<chrono::Utc>,
    > = DailyActivityRepository {
        database: FakeDatabase::default(),
        proxy_reads: PanickingReads,
        clock: super::support::fixed_now,
    };
    let _ = repository.key_metadata(&keys(&["key"]), None).await;
}

#[test]
fn tags_metadata_text_is_not_decoded() {
    let mut row = token_row("key");
    row.metadata = Some(tags_metadata(&["a", "b"]));
    let built = crate::daily_activity_repository::metadata::metadata_row(&row, true);
    assert_eq!(built.tags, vec!["a".to_owned(), "b".to_owned()]);
}
