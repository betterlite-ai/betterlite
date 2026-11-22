use betterlite_models::decisions::Json;

use crate::api::api_boundaries::ApiLimits;

use super::super::access::PollAccess;
use super::super::cache::PollingCacheError;
use super::super::error::PollingError;
use super::super::get_cache_key;
use super::super::store::{PollingUpdate, ResponsePollingStore};
use super::support::{self, CacheOperation, FakeCache, Watch};

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

pub(super) async fn empty_state(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_empty";
    cache.seed_bytes(&get_cache_key(id), &[]);
    assert_eq!(
        store
            .get_state(id, PollAccess::AuthDisabled)
            .await
            .expect("the empty value reads as absent"),
        None
    );
    let update = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"in_progress"}"#),
        )
        .await
        .expect("the empty value is a no-op");
    assert_eq!(update, PollingUpdate::MissingState);
    assert!(
        store
            .cancel_polling(id, PollAccess::AuthDisabled)
            .await
            .expect("the empty cancel still succeeds")
    );
    assert_eq!(cache.count(CacheOperation::Write), 0);
}

pub(super) async fn corrupt_state(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_corrupt";
    cache.seed(&get_cache_key(id), r#"{"id":"#);
    let error = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect_err("corrupt JSON is an error");
    assert!(matches!(error, PollingError::Decode(_)));
    let error = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"completed"}"#),
        )
        .await
        .expect_err("corrupt JSON is an error for updates too");
    assert!(matches!(error, PollingError::Decode(_)));
    assert_eq!(cache.count(CacheOperation::Write), 0);
}

pub(super) async fn present_state(cache: &FakeCache, store: &ResponsePollingStore) {
    let id = "litellm_poll_present";
    cache.seed(
        &get_cache_key(id),
        r#"{"id":"litellm_poll_present","object":"response","status":"in_progress","output":[]}"#,
    );
    let reads = cache.count(CacheOperation::Read);
    let state = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect("the stored state reads")
        .expect("the key exists");
    assert_eq!(state.get("status"), Some(&support::json("\"in_progress\"")));
    assert_eq!(cache.count(CacheOperation::Read), reads.saturating_add(1));
    assert_eq!(cache.count(CacheOperation::Write), 0);
    assert!(cache.effects().last().expect("the read was recorded").hit);
    assert!(
        store
            .cancel_polling(id, PollAccess::AuthDisabled)
            .await
            .expect("the present cancel writes")
    );
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
    cache.seed(
        &get_cache_key(id),
        r#"{"id":"litellm_poll_failures","status":"in_progress","output":[]}"#,
    );
    cache.fail_next(CacheOperation::Read);
    let error = store
        .get_state(id, PollAccess::AuthDisabled)
        .await
        .expect_err("the read error propagates");
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Read { .. })
    ));
    cache.fail_next(CacheOperation::Read);
    let error = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"completed"}"#),
        )
        .await
        .expect_err("the update read error propagates");
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Read { .. })
    ));
    cache.fail_next(CacheOperation::Write);
    let error = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"completed"}"#),
        )
        .await
        .expect_err("the write error propagates");
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Write { .. })
    ));
    cache.fail_next(CacheOperation::Delete);
    let error = store
        .delete_polling(id)
        .await
        .expect_err("the delete error propagates");
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Delete { .. })
    ));
    cache.fail_next(CacheOperation::Write);
    let error = store
        .create_initial_state(support::new_request(id, None))
        .await
        .expect_err("the create write error propagates");
    assert!(matches!(
        error,
        PollingError::Cache(PollingCacheError::Write { .. })
    ));
}

pub(super) async fn absent_cache() {
    let store = ResponsePollingStore::new(None, 3600, ApiLimits::default());
    let id = "litellm_poll_no_cache";
    assert_eq!(
        store
            .get_state(id, PollAccess::AuthDisabled)
            .await
            .expect("no cache reads as absent"),
        None
    );
    let update = store
        .update_state(
            id,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"in_progress"}"#),
        )
        .await
        .expect("no cache skips the update");
    assert_eq!(update, PollingUpdate::NoCache);
    assert!(
        store
            .cancel_polling(id, PollAccess::AuthDisabled)
            .await
            .expect("no cache still cancels")
    );
    assert!(
        !store
            .delete_polling(id)
            .await
            .expect("no cache has nothing to delete")
    );
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
