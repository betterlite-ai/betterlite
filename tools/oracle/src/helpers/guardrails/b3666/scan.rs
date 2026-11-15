use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_guardrails::guardrail_scan_cache::{ScanCacheStore, SessionScanCache as _};
use serde_json::{Value, json};

use super::{Data, Settings, guardrail_of};

const SCAN_TTL_SECONDS: i64 = 3600;

#[derive(Default)]
struct CacheState {
    entries: BTreeMap<String, Vec<String>>,
    last: Vec<String>,
}

// Stands in for DualCache and keeps the last list written, as the Python probe does
#[derive(Default)]
struct Store {
    state: Mutex<CacheState>,
}

impl ScanCacheStore for Store {
    type Error = Infallible;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<String>>, Infallible>> + Send {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(state.entries.get(key).cloned()))
    }

    fn set(
        &self,
        key: &str,
        hashes: Vec<String>,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), Infallible>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.last.clone_from(&hashes);
        state.entries.insert(key.to_owned(), hashes);
        drop(state);
        ready(Ok(()))
    }
}

// A cache that already holds the hashes of `texts`, written by a guardrail that scans and does not mask
async fn seeded(settings: &Settings, texts: &[String], data: &Data) -> Store {
    let store = Store::default();
    let mut seeding = guardrail_of(settings);
    seeding.settings.only_scan_new_messages = true;
    seeding.settings.mask_request_content = false;
    seeding.settings.mask_response_content = false;
    seeding
        .mark_texts_scanned(Some(texts), data, &store, SCAN_TTL_SECONDS)
        .await;
    store
}

// Seeds the cache with `seed`, then either marks `texts` as scanned (and answers the list written) or filters them
pub(super) async fn scan(
    settings: &Settings,
    texts: Option<&[String]>,
    seed: &[String],
    data: &Data,
    marking: bool,
) -> Value {
    let store = seeded(settings, seed, data).await;
    let guardrail = guardrail_of(settings);
    if marking {
        guardrail
            .mark_texts_scanned(texts, data, &store, SCAN_TTL_SECONDS)
            .await;
        let last = store
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last
            .clone();
        return json!(last);
    }
    json!(
        guardrail
            .filter_new_texts_for_session(texts, data, &store)
            .await
    )
}
