use betterlite_db::daily_activity_contracts::{
    DailyActivityProxyReads, KeyMetadataRow, SpendLogsWindow,
};
use betterlite_db::key_attribution::history::recovery::SpendLogHistory;
use betterlite_db::key_attribution::history::window::HistoryWindow;
use betterlite_db::key_attribution::metadata::KeyMetadata;
use betterlite_db::key_attribution::recovery::{
    attach_user_details, recover_cli_session_keys, recover_daily_spend_owners,
    recover_double_hashed_keys,
};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, NaiveDateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::future::Future;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::metadata::metadata_with_recovered_owner;

pub struct ProxyDailyActivityReads {
    pub store: PostgresStore,
    pub history: Arc<SpendLogHistory>,
}

impl DailyActivityProxyReads for ProxyDailyActivityReads {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> + Send {
        async move {
            let mut combined: BTreeMap<String, KeyMetadata> = resolved
                .iter()
                .map(|(key, row)| {
                    let metadata = KeyMetadata {
                        key_alias: row.key_alias.clone(),
                        team_id: row.team_id.clone(),
                        user_id: row.user_id.clone(),
                        user_email: row.user_email.clone(),
                        key_exists: Some(row.key_exists),
                    };
                    (key.clone(), metadata)
                })
                .collect();
            let missing = unresolved(api_keys, &combined);
            let session = recover_or_default(
                "cli_session",
                recover_cli_session_keys(&self.store, &missing).await,
            );
            combined.extend(session);
            let missing = unresolved(api_keys, &combined);
            if !missing.is_empty() {
                let reverse_hash = recover_or_default(
                    "double_hash",
                    recover_double_hashed_keys(&self.store, &missing).await,
                );
                combined.extend(reverse_hash);
            }
            let missing = unresolved(api_keys, &combined);
            if !missing.is_empty()
                && let Some(window) = window
            {
                let spend_logs = self.spend_log_metadata(&missing, window).await;
                combined.extend(spend_logs);
            }
            let ownerless = ownerless_keys(api_keys, &combined);
            let owners = recover_or_default(
                "daily_spend_owner",
                recover_daily_spend_owners(&self.store, &ownerless).await,
            );
            apply_owners(&mut combined, owners);
            let attached = match attach_user_details(&self.store, combined.clone()).await {
                Ok(attached) => attached,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "user detail attachment failed; continuing without attached details"
                    );
                    combined
                }
            };
            rows_from(&resolved, attached)
        }
    }
}

impl ProxyDailyActivityReads {
    async fn spend_log_metadata(
        &self,
        missing: &BTreeSet<String>,
        window: SpendLogsWindow,
    ) -> BTreeMap<String, KeyMetadata> {
        let (Some(start), Some(end)) = (naive(window.0), naive(window.1)) else {
            tracing::warn!(
                "the spend-log window is outside the supported time range; skipping spend-log recovery"
            );
            return BTreeMap::new();
        };
        let history = HistoryWindow { start, end };
        recover_or_default(
            "spend_logs",
            self.history
                .recover_key_metadata(&self.store, missing, &history)
                .await,
        )
    }
}

fn recover_or_default<Step, Value>(
    step: &'static str,
    result: Result<BTreeMap<String, Value>, Step>,
) -> BTreeMap<String, Value>
where
    Step: Display,
{
    match result {
        Ok(recovered) => recovered,
        Err(error) => {
            tracing::warn!(
                step = step,
                %error,
                "key metadata recovery failed; continuing without its evidence"
            );
            BTreeMap::new()
        }
    }
}

fn unresolved(
    api_keys: &BTreeSet<String>,
    resolved: &BTreeMap<String, KeyMetadata>,
) -> BTreeSet<String> {
    api_keys
        .iter()
        .filter(|key| !resolved.contains_key(*key))
        .cloned()
        .collect()
}

fn ownerless_keys(
    api_keys: &BTreeSet<String>,
    resolved: &BTreeMap<String, KeyMetadata>,
) -> BTreeSet<String> {
    api_keys
        .iter()
        .filter(|key| {
            resolved.get(*key).is_none_or(|metadata| {
                let ownerless = metadata.user_id.as_deref().is_none_or(str::is_empty);
                ownerless && !metadata.key_exists.unwrap_or(false)
            })
        })
        .cloned()
        .collect()
}

fn apply_owners(combined: &mut BTreeMap<String, KeyMetadata>, owners: BTreeMap<String, String>) {
    for (key, owner) in owners {
        let merged = metadata_with_recovered_owner(combined, &key, &owner);
        combined.insert(key, merged);
    }
}

fn rows_from(
    resolved: &BTreeMap<String, KeyMetadataRow>,
    metadata: BTreeMap<String, KeyMetadata>,
) -> BTreeMap<String, KeyMetadataRow> {
    metadata
        .into_iter()
        .map(|(key, value)| {
            let (api_key, tags) = resolved.get(&key).map_or_else(
                || (key.clone(), Vec::new()),
                |row| (row.api_key.clone(), row.tags.clone()),
            );
            let row = KeyMetadataRow {
                api_key,
                key_alias: value.key_alias,
                team_id: value.team_id,
                user_id: value.user_id,
                user_email: value.user_email,
                key_exists: value.key_exists.unwrap_or(false),
                tags,
            };
            (key, row)
        })
        .collect()
}

fn naive(time: SystemTime) -> Option<NaiveDateTime> {
    let (seconds, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (
            i64::try_from(elapsed.as_secs()).ok()?,
            elapsed.subsec_nanos(),
        ),
        Err(before) => {
            let elapsed = before.duration();
            let seconds = i64::try_from(elapsed.as_secs()).ok()?.checked_neg()?;
            if elapsed.subsec_nanos() == 0 {
                (seconds, 0)
            } else {
                (
                    seconds.checked_sub(1)?,
                    1_000_000_000_u32.checked_sub(elapsed.subsec_nanos())?,
                )
            }
        }
    };
    DateTime::<Utc>::from_timestamp(seconds, nanos).map(|moment| moment.naive_utc())
}
