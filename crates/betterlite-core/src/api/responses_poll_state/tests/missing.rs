use betterlite_models::decisions::Json;

use crate::api::api_boundaries::ApiLimits;

use super::super::access::PollAccess;
use super::super::cache::PollingCacheError;
use super::super::error::PollingError;
use super::super::get_cache_key;
use super::super::store::{PollingUpdate, ResponsePollingStore};
use super::support::{self, CacheOperation, FakeCache, Watch};

const PRESENT: &str = concat!(
    r#"{"id":"litellm_poll_present","object":"response","#,
    r#""status":"in_progress","output":[]}"#
);
const FAILING: &str = r#"{"id":"litellm_poll_failures","status":"in_progress","output":[]}"#;
const COMPLETED: &str = r#"{"status":"completed"}"#;

pub(super) async fn missing_state(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_missing";
    assert_eq!(
        store
            .get_state(id, PollAccess::AuthDisabled)
            .await
            .expect("the miss is not an error"),
        None
    );
    let update = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"in_progress"}"#),
        )
        .await
        .expect("the miss is a no-op");
    assert_eq!(update, PollingUpdate::MissingState);
    assert!(
        store
            .cancel_polling(id, PollAccess::AuthDisabled)
            .await
            .expect("the missing cancel still succeeds")
    );
    assert!(
        store
            .delete_polling(id)
            .await
            .expect("the missing delete still succeeds")
    );
    assert_missing_counts(cache);
}

fn assert_missing_counts(cache: &FakeCache) {
    assert_eq!(cache.count(CacheOperation::Write), 0);
    assert_eq!(cache.count(CacheOperation::Read), 3);
    assert_eq!(cache.count(CacheOperation::Delete), 1);
    assert!(
        cache
            .effects()
            .iter()
            .filter(|effect| effect.operation == CacheOperation::Read)
            .all(|effect| !effect.hit)
    );
}

pub(super) async fn unreadable_states(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_empty";
    cache.seed_bytes(&get_cache_key(id), &[]);
    assert_empty_value_absent(store, id).await;
    assert_corrupt_values(cache, store).await;
    assert_eq!(cache.count(CacheOperation::Write), 0);
}

async fn assert_empty_value_absent(store: &ResponsePollingStore, id: &str) {
    let state = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect("the empty value reads as absent");
    assert!(state.is_none());
    let update = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"in_progress"}"#),
        )
        .await
        .expect("the empty value is a no-op");
    assert_eq!(update, PollingUpdate::MissingState);
    let cancelled = store
        .cancel_polling(id, PollAccess::AuthDisabled)
        .await
        .expect("the empty cancel still succeeds");
    assert!(cancelled);
}

async fn assert_corrupt_values(cache: &FakeCache, store: &ResponsePollingStore) {
    let corrupt = "litellm_poll_corrupt";
    for raw in [r#"{"id":"#, r#"[{"id":"one"}]"#, "NaN", r#""text""#] {
        cache.seed(&get_cache_key(corrupt), raw);
        let error = store
            .get_state(corrupt, PollAccess::AuthDisabled)
            .await
            .expect_err("an unreadable cached value is a decode error");
        assert!(matches!(error, PollingError::Decode(_)));
    }
    let error = store
        .update_state(
            corrupt,
            PollAccess::AuthDisabled,
            &support::patch(COMPLETED),
        )
        .await
        .expect_err("an unreadable cached value fails the update too");
    assert!(matches!(error, PollingError::Decode(_)));
}

// deviation: the source raises KeyError or TypeError while it logs; betterlite logs and keeps the write
pub(super) async fn untyped_state_update(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_untyped";
    cache.seed(&get_cache_key(id), r#"{"output":{"not":"a list"}}"#);
    let update = store
        .update_state(id, PollAccess::AuthDisabled, &support::patch(COMPLETED))
        .await
        .expect("a state without status or with a non-list output still updates");
    assert_eq!(update, PollingUpdate::Updated);
    let written = support::stored(cache, id);
    assert_eq!(
        support::field(&written, "status"),
        support::json("\"completed\"")
    );
    assert_eq!(
        cache.effects().last().expect("the update wrote").ttl,
        Some(3600)
    );
}

pub(super) async fn present_state(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_present";
    cache.seed(&get_cache_key(id), PRESENT);
    assert_present_read(cache, store, id).await;
    assert_present_cancel(cache, store, id).await;
    assert_present_delete(cache, store, id).await;
}

async fn assert_present_read(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    let reads = cache.count(CacheOperation::Read);
    let writes = cache.count(CacheOperation::Write);
    let state = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect("the stored state reads")
        .expect("the key exists");
    assert_eq!(state.get("status"), Some(&support::json("\"in_progress\"")));
    assert_eq!(cache.count(CacheOperation::Read), reads.saturating_add(1));
    assert_eq!(cache.count(CacheOperation::Write), writes);
    assert!(cache.effects().last().expect("the read was recorded").hit);
}

async fn assert_present_cancel(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    let cancelled = store
        .cancel_polling(id, PollAccess::AuthDisabled)
        .await
        .expect("the present cancel writes");
    assert!(cancelled);
    let written = support::stored(cache, id);
    assert_eq!(
        support::field(&written, "status"),
        support::json("\"cancelled\"")
    );
    assert_eq!(
        written,
        support::json(concat!(
            r#"{"id":"litellm_poll_present","object":"response","#,
            r#""status":"cancelled","output":[]}"#
        ))
    );
    let effects = cache.effects();
    let last = effects.last().expect("the cancel wrote");
    assert_eq!(last.operation, CacheOperation::Write);
    assert_eq!(last.ttl, Some(3600));
}

async fn assert_present_delete(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    assert!(store.delete_polling(id).await.expect("the delete succeeds"));
    assert!(cache.entry(&get_cache_key(id)).is_none());
    assert!(
        store
            .delete_polling(id)
            .await
            .expect("a delete without a key still succeeds")
    );
}

pub(super) async fn failing_cache(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_failures";
    cache.seed(&get_cache_key(id), FAILING);
    assert_read_failure(cache, store, id).await;
    assert_read_failure_on_update(cache, store, id).await;
    assert_write_failure_on_update(cache, store, id).await;
    assert_write_failure_on_create(cache, store, id).await;
    assert_delete_failure(cache, store, id).await;
}

async fn complete(store: &ResponsePollingStore, id: &str) -> Result<PollingUpdate, PollingError> {
    store
        .update_state(id, PollAccess::AuthDisabled, &support::patch(COMPLETED))
        .await
}

fn expect_read_error(error: PollingError) {
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Read(_))
    ));
}

fn expect_write_error(error: PollingError) {
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Write(_))
    ));
}

fn expect_delete_error(error: PollingError) {
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Delete(_))
    ));
}

async fn assert_read_failure(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    cache.fail_next(CacheOperation::Read);
    let result = store.get_state(id, PollAccess::AuthDisabled).await;
    expect_read_error(result.expect_err("the read failure propagates"));
}

async fn assert_read_failure_on_update(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    cache.fail_next(CacheOperation::Read);
    let result = complete(store, id).await;
    expect_read_error(result.expect_err("the read failure propagates"));
}

async fn assert_write_failure_on_update(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    cache.fail_next(CacheOperation::Write);
    let result = complete(store, id).await;
    expect_write_error(result.expect_err("the write failure propagates"));
}

async fn assert_write_failure_on_create(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    cache.fail_next(CacheOperation::Write);
    let result = store
        .create_initial_state(support::new_request(id, None))
        .await;
    expect_write_error(result.expect_err("the write failure propagates"));
}

async fn assert_delete_failure(cache: &FakeCache, store: &ResponsePollingStore, id: &str) {
    cache.fail_next(CacheOperation::Delete);
    let result = store.delete_polling(id).await;
    expect_delete_error(result.expect_err("the delete failure propagates"));
}

pub(super) async fn absent_cache() {
    let store = ResponsePollingStore {
        cache: None,
        ttl: 3600,
        limits: ApiLimits::default(),
    };
    let id = "litellm_poll_no_cache";
    assert_absent_reads(&store, id).await;
    assert_absent_writes(&store, id).await;
}

async fn assert_absent_reads(store: &ResponsePollingStore, id: &str) {
    let state = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect("no cache reads as absent");
    assert!(state.is_none());
}

async fn assert_absent_writes(store: &ResponsePollingStore, id: &str) {
    let update = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"in_progress"}"#),
        )
        .await
        .expect("no cache skips the update");
    assert_eq!(update, PollingUpdate::NoCache);
    let cancelled = store
        .cancel_polling(id, PollAccess::AuthDisabled)
        .await
        .expect("no cache still cancels");
    assert!(cancelled);
    let deleted = store
        .delete_polling(id)
        .await
        .expect("no cache has nothing to delete");
    assert!(!deleted);
    let response = store
        .create_initial_state(support::new_request(id, None))
        .await
        .expect("no cache still returns the queued response");
    assert_eq!(
        support::field(&Json::Object(response), "status"),
        support::json("\"queued\"")
    );
}

pub(super) fn assert_logs(watch: &Watch) {
    assert_eq!(
        watch.messages_at("WARNING"),
        vec![
            "No cached state found for polling_id: litellm_poll_missing",
            "No cached state found for polling_id: litellm_poll_missing",
            "No cached state found for polling_id: litellm_poll_empty",
            "No cached state found for polling_id: litellm_poll_empty",
        ]
    );
}
