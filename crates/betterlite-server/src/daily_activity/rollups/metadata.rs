use std::collections::{BTreeMap, BTreeSet};

use betterlite_analytics::daily_metrics::KeyMetadata;
use betterlite_db::daily_activity_contracts::{
    EntityRollupRow, GroupingSetsRow, KeyMetadataRow, RollupMetricsRow,
};
use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::EntityMetadata;

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

pub(super) fn metadata_keys<Record: RecordMetrics>(records: &[Record]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for record in records {
        if let Some(api_key) = non_sentinel_key(record.metrics_row()) {
            keys.insert(api_key.to_owned());
        }
    }
    keys
}

pub(super) fn record_dates<Record: RecordMetrics>(records: &[Record]) -> BTreeSet<String> {
    let mut dates = BTreeSet::new();
    for record in records {
        if let Some(date) = record.metrics_row().date.as_ref() {
            dates.insert(date.clone());
        }
    }
    dates
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
