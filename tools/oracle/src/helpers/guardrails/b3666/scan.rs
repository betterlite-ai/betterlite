use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_guardrails::guardrail_scan_cache::ScanCacheStore;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{Data, Settings, guardrail_of};

const SCAN_TTL_SECONDS: i64 = 3600;

// Stands in for DualCache and keeps the last list written, as the Python probe does
#[derive(Default)]
struct Store {
    entries: Mutex<BTreeMap<String, Vec<String>>>,
    last: Mutex<Vec<String>>,
}

impl ScanCacheStore for Store {
    type Error = Infallible;

    async fn get(&self, key: &str) -> Result<Option<Vec<String>>, Infallible> {
        Ok(self.entries.lock().await.get(key).cloned())
    }

    async fn set(
        &self,
        key: &str,
        hashes: Vec<String>,
        _ttl_seconds: i64,
    ) -> Result<(), Infallible> {
        self.last.lock().await.clone_from(&hashes);
        self.entries.lock().await.insert(key.to_owned(), hashes);
        Ok(())
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

pub(super) async fn filter(
    settings: &Settings,
    texts: Option<&[String]>,
    cached: &[String],
    data: &Data,
) -> Value {
    let store = seeded(settings, cached, data).await;
    let filtered = guardrail_of(settings)
        .filter_new_texts_for_session(texts, data, &store)
        .await;
    json!(filtered)
}

pub(super) async fn mark(
    settings: &Settings,
    texts: Option<&[String]>,
    existing: &[String],
    data: &Data,
) -> Value {
    let store = seeded(settings, existing, data).await;
    guardrail_of(settings)
        .mark_texts_scanned(texts, data, &store, SCAN_TTL_SECONDS)
        .await;
    let last = store.last.lock().await.clone();
    json!(last)
}
