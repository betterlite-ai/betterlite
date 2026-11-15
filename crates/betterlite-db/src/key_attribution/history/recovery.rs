use std::collections::{BTreeMap, BTreeSet};

use tokio::sync::Mutex;

use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::session::PostgresStore;

use crate::key_attribution::bounded::bounded_rows;
use crate::key_attribution::metadata::KeyMetadata;

use super::cache::MetadataCache;
use super::error::HistoryError;
use super::rows::{edge_metadata, spend_log_values};
use super::sql::{NO_BITMAP_SCAN_SQL, spend_log_alias_sql};
use super::window::{HistoryWindow, is_spend_log_digest};

#[derive(Debug, Default)]
pub struct SpendLogHistory {
    pub cache: MetadataCache,
    pub query_lock: Mutex<()>,
}

impl SpendLogHistory {
    pub async fn recover_key_metadata(
        &self,
        store: &PostgresStore,
        missing_keys: &BTreeSet<String>,
        window: &HistoryWindow,
    ) -> Result<BTreeMap<String, KeyMetadata>, HistoryError> {
        let digests: BTreeSet<String> = missing_keys
            .iter()
            .filter(|key| is_spend_log_digest(key))
            .cloned()
            .collect();
        if digests.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut recovered = self.cache.cached(&digests, window);
        let uncached: BTreeSet<String> = digests
            .iter()
            .filter(|digest| !recovered.contains_key(*digest))
            .cloned()
            .collect();
        if !uncached.is_empty() {
            recovered.extend(self.query_one_at_a_time(store, &uncached, window).await?);
        }
        recovered.retain(|_, metadata| *metadata != KeyMetadata::default());
        Ok(recovered)
    }

    async fn query_one_at_a_time(
        &self,
        store: &PostgresStore,
        digests: &BTreeSet<String>,
        window: &HistoryWindow,
    ) -> Result<BTreeMap<String, KeyMetadata>, HistoryError> {
        let _serialized = self.query_lock.lock().await;
        let mut settled = self.cache.cached(digests, window);
        let pending: BTreeSet<String> = digests
            .iter()
            .filter(|digest| !settled.contains_key(*digest))
            .cloned()
            .collect();
        let found = if pending.is_empty() {
            BTreeMap::new()
        } else {
            query_digests(store, &pending, window)
                .await?
                .unwrap_or_default()
        };
        for digest in &pending {
            self.cache.remember(digest, window, found.get(digest));
        }
        settled.extend(found);
        Ok(settled)
    }
}

async fn query_digests(
    store: &PostgresStore,
    digests: &BTreeSet<String>,
    window: &HistoryWindow,
) -> Result<Option<BTreeMap<String, KeyMetadata>>, HistoryError> {
    let values: Vec<String> = digests.iter().cloned().collect();
    let sql = spend_log_alias_sql();
    let loaded = bounded_rows(
        store,
        &sql,
        &[&values, &window.start, &window.end],
        &[NO_BITMAP_SCAN_SQL],
    )
    .await
    .inspect_err(|error| {
        tracing::warn!(
            keys = digests.len(),
            %error,
            "spend-log key recovery failed; continuing without historical evidence"
        );
    });
    let Ok(rows) = loaded else {
        return Ok(None);
    };
    let decoded = rows
        .iter()
        .map(spend_log_values)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|driver| HistoryError::Row {
            source: DriverError { driver },
        })?;
    Ok(Some(edge_metadata(&decoded, digests)))
}
