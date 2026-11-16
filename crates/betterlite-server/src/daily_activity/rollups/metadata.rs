use std::collections::{BTreeMap, BTreeSet};

use betterlite_analytics::daily_metrics::KeyMetadata;
use betterlite_db::daily_activity_contracts::{
    EntityRollupRow, GroupingSetsRow, KeyMetadataRow, RollupMetricsRow,
};
use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{AggregatedActivityRepository, DailyActivityAggregateError, EntityMetadata};
use crate::daily_activity::recovery::window::spend_logs_window;

pub(super) const UNASSIGNED_ENTITY: &str = "Unassigned";

pub(super) const fn entity_name(entity_id: Option<&str>) -> &str {
    // the entity query coalesces a null id to an empty string
    match entity_id {
        Some(value) if !value.is_empty() => value,
        _ => UNASSIGNED_ENTITY,
    }
}

pub(super) trait RecordMetrics {
    fn metrics_row(&self) -> &RollupMetricsRow;
}

impl RecordMetrics for GroupingSetsRow {
    fn metrics_row(&self) -> &RollupMetricsRow {
        &self.metrics
    }
}

impl RecordMetrics for EntityRollupRow {
    fn metrics_row(&self) -> &RollupMetricsRow {
        &self.metrics
    }
}

pub(super) fn non_sentinel_key(metrics: &RollupMetricsRow) -> Option<&str> {
    match metrics.api_key.as_deref() {
        Some(api_key) if api_key != PTU_SENTINEL_API_KEY => Some(api_key),
        _ => None,
    }
}

pub(super) struct KeyLookup {
    pub(super) api_keys: BTreeSet<String>,
    pub(super) dates: BTreeSet<String>,
}

pub(super) fn key_lookup<Record>(records: &[Record]) -> KeyLookup
where
    Record: RecordMetrics,
{
    let mut lookup = KeyLookup {
        api_keys: BTreeSet::new(),
        dates: BTreeSet::new(),
    };
    for record in records {
        let metrics = record.metrics_row();
        if let Some(api_key) = non_sentinel_key(metrics) {
            lookup.api_keys.insert(api_key.to_owned());
        }
        if let Some(date) = metrics.date.as_ref() {
            lookup.dates.insert(date.clone());
        }
    }
    lookup
}

pub(super) async fn fetch_key_metadata<Repository>(
    repository: &Repository,
    lookup: &KeyLookup,
) -> Result<BTreeMap<String, KeyMetadataRow>, DailyActivityAggregateError>
where
    Repository: AggregatedActivityRepository,
{
    if lookup.api_keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let window = spend_logs_window(&lookup.dates);
    Ok(repository
        .read_key_metadata(&lookup.api_keys, window)
        .await?)
}

pub(super) fn key_metadata(
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
    api_key: &str,
) -> KeyMetadata {
    let Some(row) = api_key_metadata.get(api_key) else {
        return KeyMetadata {
            key_alias: None,
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: Some(false),
        };
    };
    KeyMetadata {
        key_alias: row.key_alias.clone(),
        team_id: row.team_id.clone(),
        user_id: row.user_id.clone(),
        user_email: row.user_email.clone(),
        key_exists: Some(row.key_exists),
    }
}

pub(super) fn entity_metadata(
    entity_metadata_field: Option<&EntityMetadata>,
    entity_id: &str,
) -> IndexMap<String, Json> {
    entity_metadata_field
        .and_then(|field| field.get(entity_id))
        .cloned()
        .unwrap_or_default()
}
