#![cfg(test)]

use std::fs;
use std::sync::Arc;

use crate::api::api_boundaries::ApiLimits;

use self::support::FakeCache;
use super::decode::decode_responses_poll_state;

mod missing;
mod owners;
mod patching;
mod queued;
mod selection;
mod support;

#[tokio::test]
async fn queued_record_and_cache_key() {
    support::keys_and_ids();
    queued::queued_record().await;
    queued::queued_without_metadata().await;
    queued::queued_metadata_null().await;
    queued::queued_without_cache().await;
}

#[tokio::test]
async fn polling_patch_truthiness() {
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), patching::TTL);
    patching::seed(&cache);
    let (guard, watch) = support::watch();
    patching::falsy_patches(&cache, &store).await;
    patching::error_overrides_status(&cache, &store).await;
    patching::present_fields(&cache, &store).await;
    patching::null_patch_leaves_fields(&cache, &store).await;
    patching::ttl_is_refreshed(&cache);
    drop(guard);
    patching::assert_logs(&watch);
}

#[tokio::test]
async fn polling_missing_cancel_delete() {
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), 3600);
    let (guard, watch) = support::watch();
    missing::missing_state(&cache, &store).await;
    missing::empty_state(&cache, &store).await;
    missing::present_state(&cache, &store).await;
    missing::corrupt_state(&cache, &store).await;
    missing::failing_cache(&cache, &store).await;
    missing::absent_cache().await;
    drop(guard);
    missing::assert_logs(&watch);
}

#[test]
fn polling_selection() {
    let (guard, watch) = support::watch();
    selection::requires_conditions();
    selection::native_mode();
    selection::prefixed_models();
    selection::alias_scan();
    selection::bad_index();
    drop(guard);
    selection::assert_logs(&watch);
}

#[tokio::test]
async fn polling_owner_retention() {
    owners::identity_derivation();
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), 3600);
    owners::owner_retention(&cache, &store).await;
    owners::identity_denials(&cache, &store).await;
    owners::fingerprint_denials(&cache, &store).await;
    owners::record_shape_denials(&cache, &store).await;
}

#[test]
fn replays_corpus() {
    let corpus = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/responses_poll_state"
    );
    let mut accepted = 0_usize;
    let mut rejected = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory exists") {
        let path = entry.expect("the corpus entry reads").path();
        let bytes = fs::read(&path).expect("the seed reads");
        match decode_responses_poll_state(&bytes, ApiLimits::default()) {
            Ok(_state) => accepted = accepted.saturating_add(1),
            Err(_error) => rejected = rejected.saturating_add(1),
        }
    }
    assert!(accepted > 0, "a stored polling record decodes");
    assert!(rejected > 0, "a malformed seed is rejected");
}
