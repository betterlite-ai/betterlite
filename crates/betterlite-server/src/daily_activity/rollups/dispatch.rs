use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry as BTreeEntry;

use betterlite_analytics::canonical_date::parse_canonical_date;
use betterlite_analytics::daily_metrics::{
    BreakdownMetrics, DailySpendData, KeyMetricWithMetadata, MetricWithMetadata, SpendMetrics,
};
use betterlite_db::daily_activity_contracts::{GroupingSetsRow, KeyMetadataRow, RollupMetricsRow};
use indexmap::IndexMap;
use indexmap::map::Entry;
use rust_decimal::Decimal;
use tokio::task::JoinSet;

use super::levels::{Dimension, RollupLevel, rollup_level};
use super::metadata::{key_metadata, metadata_keys, non_sentinel_key, record_dates};
use super::{AggregatedActivityRepository, AggregatedSpendData, DailyActivityAggregateError};
use crate::daily_activity::recovery::window::spend_logs_window;

#[derive(Debug, Default)]
pub(super) struct GroupedDay {
    pub(super) metrics: SpendMetrics,
    pub(super) breakdown: BreakdownMetrics,
}

pub(super) fn aggregate_grouping_rows_sync(
    records: &[GroupingSetsRow],
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
) -> Result<AggregatedSpendData, DailyActivityAggregateError> {
    let mut totals = SpendMetrics::default();
    let mut grouped: BTreeMap<String, GroupedDay> = BTreeMap::new();
    for record in records {
        let level = rollup_level(record.group_level);
        let metrics = record_to_spend_metrics(&record.metrics);
        if matches!(level, RollupLevel::GrandTotal) {
            totals = metrics;
            continue;
        }
        let Some(raw_date) = record.metrics.date.as_deref() else {
            return Err(DailyActivityAggregateError::MissingDate);
        };
        let day = ensure_date(&mut grouped, raw_date);
        if matches!(level, RollupLevel::Daily) {
            day.metrics = metrics;
            continue;
        }
        assign_level(&mut day.breakdown, level, record, metrics, api_key_metadata);
    }
    let mut results = Vec::with_capacity(grouped.len());
    for (raw_date, day) in grouped {
        let Some(date) = parse_canonical_date(&raw_date) else {
            return Err(DailyActivityAggregateError::Date { value: raw_date });
        };
        results.push(DailySpendData {
            date,
            metrics: day.metrics,
            breakdown: day.breakdown,
        });
    }
    results.sort_by_key(|day| Reverse(day.date));
    Ok(AggregatedSpendData { results, totals })
}

pub(super) async fn aggregate_grouping_rows<Repository>(
    repository: &Repository,
    records: Vec<GroupingSetsRow>,
) -> Result<AggregatedSpendData, DailyActivityAggregateError>
where
    Repository: AggregatedActivityRepository,
{
    let api_keys = metadata_keys(&records);
    let api_key_metadata = if api_keys.is_empty() {
        BTreeMap::new()
    } else {
        let window = spend_logs_window(&record_dates(&records));
        repository.read_key_metadata(&api_keys, window).await?
    };
    run_worker(move || aggregate_grouping_rows_sync(&records, &api_key_metadata)).await?
}

pub(super) fn ensure_date<'days>(
    grouped: &'days mut BTreeMap<String, GroupedDay>,
    date: &str,
) -> &'days mut GroupedDay {
    match grouped.entry(date.to_owned()) {
        BTreeEntry::Occupied(entry) => entry.into_mut(),
        BTreeEntry::Vacant(entry) => entry.insert(GroupedDay::default()),
    }
}

fn assign_level(
    breakdown: &mut BreakdownMetrics,
    level: RollupLevel,
    record: &GroupingSetsRow,
    metrics: SpendMetrics,
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
) {
    match level {
        RollupLevel::GrandTotal | RollupLevel::Daily | RollupLevel::Other => {}
        RollupLevel::ApiKeys => {
            assign_api_key(breakdown, record, metrics, api_key_metadata);
        }
        RollupLevel::Dimension {
            dimension,
            zero_flat_cost,
        } => {
            assign_dimension(breakdown, dimension, zero_flat_cost, record, metrics);
        }
        RollupLevel::Keyed {
            dimension,
            sentinel_filtered,
        } => {
            let Some(name) = dimension.name(record) else {
                return;
            };
            let api_key = if sentinel_filtered {
                non_sentinel_key(&record.metrics)
            } else {
                record.metrics.api_key.as_deref()
            };
            let Some(key) = api_key else {
                return;
            };
            assign_api_key_breakdown(
                dimension.target(breakdown),
                name,
                key,
                metrics,
                api_key_metadata,
            );
        }
    }
}

fn assign_dimension(
    breakdown: &mut BreakdownMetrics,
    dimension: Dimension,
    zero_flat_cost: bool,
    record: &GroupingSetsRow,
    mut metrics: SpendMetrics,
) {
    let Some(name) = dimension.name(record) else {
        return;
    };
    if zero_flat_cost {
        metrics.flat_cost = Decimal::ZERO;
    }
    assign_metric_with_metadata(dimension.target(breakdown), name, metrics);
}

fn assign_api_key(
    breakdown: &mut BreakdownMetrics,
    record: &GroupingSetsRow,
    metrics: SpendMetrics,
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
) {
    if let Some(api_key) = non_sentinel_key(&record.metrics) {
        breakdown.api_keys.insert(
            api_key.to_owned(),
            KeyMetricWithMetadata {
                metrics,
                metadata: key_metadata(api_key_metadata, api_key),
            },
        );
    }
}

pub(super) fn assign_metric_with_metadata(
    target: &mut IndexMap<String, MetricWithMetadata>,
    name: &str,
    metrics: SpendMetrics,
) {
    match target.entry(name.to_owned()) {
        Entry::Occupied(mut entry) => entry.get_mut().metrics = metrics,
        Entry::Vacant(entry) => {
            entry.insert(MetricWithMetadata {
                metrics,
                metadata: IndexMap::new(),
                api_key_breakdown: IndexMap::new(),
            });
        }
    }
}

pub(super) fn assign_api_key_breakdown(
    target: &mut IndexMap<String, MetricWithMetadata>,
    parent_key: &str,
    api_key: &str,
    metrics: SpendMetrics,
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
) {
    let parent = target
        .entry(parent_key.to_owned())
        .or_insert_with(|| MetricWithMetadata {
            metrics: SpendMetrics::default(),
            metadata: IndexMap::new(),
            api_key_breakdown: IndexMap::new(),
        });
    parent.api_key_breakdown.insert(
        api_key.to_owned(),
        KeyMetricWithMetadata {
            metrics,
            metadata: key_metadata(api_key_metadata, api_key),
        },
    );
}

pub(super) fn record_to_spend_metrics(record: &RollupMetricsRow) -> SpendMetrics {
    let prompt_tokens = record.prompt_tokens.unwrap_or_default();
    let completion_tokens = record.completion_tokens.unwrap_or_default();
    SpendMetrics {
        spend: record.spend.unwrap_or_default(),
        flat_cost: record.ptu_flat_cost.unwrap_or_default(),
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        cache_read_input_tokens: record.cache_read_input_tokens.unwrap_or_default(),
        cache_creation_input_tokens: record.cache_creation_input_tokens.unwrap_or_default(),
        compression_saved_tokens: record.compression_saved_tokens.unwrap_or_default(),
        compression_savings_spend: record.compression_savings_spend.unwrap_or_default(),
        prompt_caching_savings_spend: record.prompt_caching_savings_spend.unwrap_or_default(),
        gateway_caching_savings: record.gateway_cache_savings_spend.unwrap_or_default(),
        autorouter_savings_spend: record.autorouter_savings_spend.unwrap_or_default(),
        api_requests: record.api_requests.unwrap_or_default(),
        successful_requests: record.successful_requests.unwrap_or_default(),
        failed_requests: record.failed_requests.unwrap_or_default(),
        total_response_time_ms: record.total_response_time_ms.unwrap_or_default(),
        timed_requests: record.timed_requests.unwrap_or_default(),
    }
}

pub(super) async fn run_worker<Value, Operation>(
    work: Operation,
) -> Result<Value, DailyActivityAggregateError>
where
    Value: Send + 'static,
    Operation: FnOnce() -> Value + Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(work);
    let joined = tasks
        .join_next()
        .await
        .ok_or(DailyActivityAggregateError::WorkerMissing)?;
    Ok(joined?)
}
