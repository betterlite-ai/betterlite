use std::collections::BTreeMap;

use betterlite_analytics::daily_metrics::{
    DailySpendData, KeyMetricWithMetadata, MetricWithMetadata, SpendMetrics,
};
use betterlite_db::daily_activity_contracts::{EntityRollupRow, KeyMetadataRow};
use indexmap::IndexMap;
use indexmap::map::Entry;

use super::dispatch::record_to_spend_metrics;
use super::metadata::{
    entity_metadata, entity_name, key_metadata, metadata_keys, non_sentinel_key, record_dates,
};
use super::{AggregatedActivityRepository, DailyActivityAggregateError, EntityMetadata};
use crate::daily_activity::recovery::window::spend_logs_window;

pub(super) async fn fold_entity_rollups<Repository>(
    repository: &Repository,
    results: &mut [DailySpendData],
    entity_rows: &[EntityRollupRow],
    entity_metadata_field: Option<&EntityMetadata>,
) -> Result<(), DailyActivityAggregateError>
where
    Repository: AggregatedActivityRepository,
{
    let api_keys = metadata_keys(entity_rows);
    let api_key_metadata = if api_keys.is_empty() {
        BTreeMap::new()
    } else {
        let window = spend_logs_window(&record_dates(entity_rows));
        repository.read_key_metadata(&api_keys, window).await?
    };
    fold_entity_rollups_sync(
        results,
        entity_rows,
        &api_key_metadata,
        entity_metadata_field,
    );
    Ok(())
}

pub(super) fn fold_entity_rollups_sync(
    results: &mut [DailySpendData],
    entity_rows: &[EntityRollupRow],
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
    entity_metadata_field: Option<&EntityMetadata>,
) {
    let mut by_date: BTreeMap<String, usize> = BTreeMap::new();
    for (index, day) in results.iter().enumerate() {
        by_date.insert(day.date.format("%Y-%m-%d").to_string(), index);
    }
    for row in entity_rows {
        let Some(raw_date) = row.metrics.date.as_deref() else {
            continue;
        };
        let Some(index) = by_date.get(raw_date) else {
            continue;
        };
        let Some(day) = results.get_mut(*index) else {
            continue;
        };
        let entities = &mut day.breakdown.entities;
        let entity_id = entity_name(row.entity_id.as_deref());
        let bucket = match entities.entry(entity_id.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(MetricWithMetadata {
                metrics: SpendMetrics::default(),
                metadata: entity_metadata(entity_metadata_field, entity_id),
                api_key_breakdown: IndexMap::new(),
            }),
        };
        assign_entity_metrics(bucket, row, api_key_metadata);
    }
}

fn assign_entity_metrics(
    bucket: &mut MetricWithMetadata,
    row: &EntityRollupRow,
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
) {
    let metrics = record_to_spend_metrics(&row.metrics);
    if row.api_key_rolled != 0 {
        bucket.metrics = metrics;
        return;
    }
    let Some(api_key) = non_sentinel_key(&row.metrics) else {
        return;
    };
    bucket.api_key_breakdown.insert(
        api_key.to_owned(),
        KeyMetricWithMetadata {
            metrics,
            metadata: key_metadata(api_key_metadata, api_key),
        },
    );
}
