use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;

use crate::auth_cache::gateway_cache;
use crate::auth_read::table::AuthReadError;
use crate::auth_resource_grants::managed_rows::{
    ManagedVectorStoreCache, ManagedVectorStoreLookupError, VectorStoreRow, project_row,
    vector_store_rows_by_uuids,
};
use crate::auth_resource_grants::tests::support::{
    RecordingCache, RecordingRows, managed_store, row,
};

fn stored(uuid: &str) -> ManagedVectorStoresTable {
    managed_store(&format!(
        r#"{{"vector_store_id":"{uuid}","custom_llm_provider":"openai"}}"#
    ))
}

fn fetched(uuid: &str) -> VectorStoreRow {
    row(&format!(
        r#"{{"model_dump":{{"vector_store_id":"{uuid}","custom_llm_provider":"openai"}}}}"#
    ))
}

fn identifiers(rows: &[ManagedVectorStoresTable]) -> Vec<&str> {
    rows.iter()
        .map(|stored| stored.vector_store_id.as_str())
        .collect()
}

#[tokio::test]
async fn no_db_or_ids_no_io() {
    let cache = RecordingCache::new(60.0);
    let rows = RecordingRows::new(Vec::new());
    assert!(
        vector_store_rows_by_uuids(&[], true, &cache, &rows)
            .await
            .expect("no ids needs no lookup")
            .is_empty()
    );
    assert!(
        vector_store_rows_by_uuids(&["a".to_owned()], false, &cache, &rows)
            .await
            .expect("no database needs no lookup")
            .is_empty()
    );
    assert!(cache.reads().is_empty());
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn ordered_cache_hits_first() {
    let cache = RecordingCache::new(60.0).with_hit("b", stored("b"));
    let rows = RecordingRows::new(vec![fetched("a"), fetched("c")]);
    let found = vector_store_rows_by_uuids(
        &["a".to_owned(), "b".to_owned(), "c".to_owned()],
        true,
        &cache,
        &rows,
    )
    .await
    .expect("the mixed hit and miss lookup");
    assert_eq!(identifiers(&found), ["b", "a", "c"]);
    assert_eq!(
        rows.queries(),
        vec![(vec!["a".to_owned(), "c".to_owned()], 2)]
    );
}

#[tokio::test]
async fn duplicate_uuid_hits_kept() {
    let cache = RecordingCache::new(60.0).with_hit("a", stored("a"));
    let rows = RecordingRows::new(Vec::new());
    let found = vector_store_rows_by_uuids(&["a".to_owned(), "a".to_owned()], true, &cache, &rows)
        .await
        .expect("the duplicated lookup");
    assert_eq!(identifiers(&found), ["a", "a"]);
    assert_eq!(
        cache.reads(),
        vec![
            "managed_vector_store_id:a".to_owned(),
            "managed_vector_store_id:a".to_owned()
        ]
    );
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn batched_misses_take_count() {
    let cache = RecordingCache::new(60.0).with_hit("b", stored("b"));
    let rows = RecordingRows::new(vec![fetched("a"), fetched("c")]);
    vector_store_rows_by_uuids(
        &["a".to_owned(), "b".to_owned(), "c".to_owned()],
        true,
        &cache,
        &rows,
    )
    .await
    .expect("the batched lookup");
    assert_eq!(
        rows.queries(),
        vec![(vec!["a".to_owned(), "c".to_owned()], 2)]
    );
}

#[test]
fn row_projection_precedence() {
    let dump = row(
        r#"{"model_dump":{"vector_store_id":"dump","custom_llm_provider":"openai"},"dict":{"vector_store_id":"dict","custom_llm_provider":"openai"},"fields":{"vector_store_id":"fields","custom_llm_provider":"openai"}}"#,
    );
    let projected_dump = project_row(&dump)
        .expect("the model_dump projection")
        .expect("the projected row");
    assert_eq!(projected_dump.vector_store_id, "dump");
    let dict_row = row(
        r#"{"dict":{"vector_store_id":"dict","custom_llm_provider":"openai"},"fields":{"vector_store_id":"fields","custom_llm_provider":"openai"}}"#,
    );
    let projected_dict = project_row(&dict_row)
        .expect("the dict projection")
        .expect("the projected row");
    assert_eq!(projected_dict.vector_store_id, "dict");
    // LiteLLM's empty model_dump() is falsy, so it falls straight to dict(row), the iterable form
    let empty_dump = row(
        r#"{"model_dump":{},"dict":{"vector_store_id":"dict","custom_llm_provider":"openai"},"fields":{"vector_store_id":"fields","custom_llm_provider":"openai"}}"#,
    );
    let projected_empty_dump = project_row(&empty_dump)
        .expect("the empty dump falls back")
        .expect("the projected row");
    assert_eq!(projected_empty_dump.vector_store_id, "fields");
    let iterable = row(
        r#"{"model_dump":null,"dict":null,"fields":{"vector_store_id":"fields","custom_llm_provider":"openai"}}"#,
    );
    let projected_fields = project_row(&iterable)
        .expect("the iterable projection")
        .expect("the projected row");
    assert_eq!(projected_fields.vector_store_id, "fields");
    let unreadable = row(r#"{"model_dump":"text","dict":{},"fields":{}}"#);
    assert!(
        project_row(&unreadable)
            .expect("the unreadable projection is skipped")
            .is_none()
    );
    let nothing = row("{}");
    assert!(
        project_row(&nothing)
            .expect("the empty row is skipped")
            .is_none()
    );
}

#[tokio::test]
async fn empty_projection_skipped() {
    let cache = RecordingCache::new(60.0);
    let rows = RecordingRows::new(vec![row("{}"), fetched("a")]);
    let found = vector_store_rows_by_uuids(&["a".to_owned(), "b".to_owned()], true, &cache, &rows)
        .await
        .expect("the empty projection is skipped");
    assert_eq!(identifiers(&found), ["a"]);
    assert_eq!(
        cache.writes(),
        vec![("managed_vector_store_id:a".to_owned(), 60.0)]
    );
}

#[tokio::test]
async fn missing_rows_omitted() {
    let cache = RecordingCache::new(60.0);
    let rows = RecordingRows::new(vec![fetched("a")]);
    let found = vector_store_rows_by_uuids(&["a".to_owned(), "b".to_owned()], true, &cache, &rows)
        .await
        .expect("the partial answer");
    assert_eq!(identifiers(&found), ["a"]);
    assert_eq!(cache.writes().len(), 1);
}

#[tokio::test]
async fn management_ttl_applied() {
    let cache = RecordingCache::new(123.5);
    let rows = RecordingRows::new(vec![fetched("a")]);
    let found = vector_store_rows_by_uuids(&["a".to_owned()], true, &cache, &rows)
        .await
        .expect("the cached write");
    assert_eq!(identifiers(&found), ["a"]);
    assert_eq!(
        cache.writes(),
        vec![("managed_vector_store_id:a".to_owned(), 123.5)]
    );
}

#[tokio::test]
async fn auth_cache_round_trip_uses_management_ttl() {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1000.0_f64);
    let memory = Arc::new(LocalMemoryOps::new(1024, clock).expect("the local memory tier builds"));
    let cache = gateway_cache(
        Arc::clone(&memory),
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: Some(30.0_f64),
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    );
    let key = "managed_vector_store_id:a";
    assert_eq!(cache.management_ttl().to_bits(), 30.0_f64.to_bits());
    assert!(
        cache
            .read(key)
            .await
            .expect("the empty cache read")
            .is_none()
    );
    cache
        .write(key, &stored("a"), cache.management_ttl())
        .await
        .expect("the cache write");
    assert_eq!(
        cache.read(key).await.expect("the cached read"),
        Some(stored("a"))
    );
}

#[tokio::test]
async fn lookup_failure_propagates() {
    let cache = RecordingCache::new(60.0);
    let rows = RecordingRows::failing();
    match vector_store_rows_by_uuids(&["a".to_owned()], true, &cache, &rows).await {
        Err(ManagedVectorStoreLookupError::Rows(failure)) => {
            assert!(matches!(
                *failure.source,
                AuthReadError::Lookup(DbLookupError::Database(DbError {
                    error_type: ErrorType::NoDbConnection,
                    ..
                }))
            ));
        }
        other => panic!("the repository failure propagates, got {other:?}"),
    }
    assert_eq!(cache.reads(), vec!["managed_vector_store_id:a".to_owned()]);
}

#[tokio::test]
async fn cache_read_failure_propagates() {
    let cache = RecordingCache::new(60.0).failing_read();
    let rows = RecordingRows::new(vec![fetched("a")]);
    match vector_store_rows_by_uuids(&["a".to_owned()], true, &cache, &rows).await {
        Err(ManagedVectorStoreLookupError::Cache(_)) => {}
        other => panic!("the cache read failure propagates, got {other:?}"),
    }
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn cache_partial_writes_survive() {
    let cache = RecordingCache::new(60.0);
    let rows = RecordingRows::new(vec![
        fetched("a"),
        row(r#"{"model_dump":{"vector_store_id":"b"}}"#),
    ]);
    match vector_store_rows_by_uuids(&["a".to_owned(), "b".to_owned()], true, &cache, &rows).await {
        Err(ManagedVectorStoreLookupError::Record(_)) => {}
        other => panic!("the invalid row stops the lookup, got {other:?}"),
    }
    assert_eq!(
        cache.writes(),
        vec![("managed_vector_store_id:a".to_owned(), 60.0)]
    );

    let failing = RecordingCache::new(60.0).failing_write_after(1);
    let two_rows = RecordingRows::new(vec![fetched("a"), fetched("b")]);
    match vector_store_rows_by_uuids(&["a".to_owned(), "b".to_owned()], true, &failing, &two_rows)
        .await
    {
        Err(ManagedVectorStoreLookupError::CacheWrite(_)) => {}
        other => panic!("the cache write failure propagates, got {other:?}"),
    }
    assert_eq!(
        failing.writes(),
        vec![("managed_vector_store_id:a".to_owned(), 60.0)]
    );
}
