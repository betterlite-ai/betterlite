use betterlite_analytics::daily_metrics::{BreakdownMetrics, DailySpendData, SpendMetrics};
use betterlite_db::daily_activity_contracts::{DailyActivityProxyReads, KeyMetadataRow};
use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::daily_activity_repository::database::DailyActivityDatabase;
use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use chrono::{DateTime, NaiveDate, Utc};
use indexmap::IndexMap;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use tokio::task::JoinSet;

use super::breakdown::{BreakdownContext, update_breakdown_metrics, update_metrics};
use super::error::PagesError;
use super::metadata::EntityMetadata;
use super::record::DailySpendRecord;
use crate::daily_activity::recovery::window::spend_logs_window;

#[derive(Debug)]
pub struct AggregatedSpend {
    pub results: Vec<DailySpendData>,
    pub totals: SpendMetrics,
}

struct DayGroup {
    metrics: SpendMetrics,
    breakdown: BreakdownMetrics,
}

pub async fn aggregate_spend_records<Database, Reads, Clock>(
    repository: &DailyActivityRepository<Database, Reads, Clock>,
    records: &[DailySpendRecord],
    entity_id_field: Option<&str>,
    entity_metadata: Option<&EntityMetadata>,
) -> Result<AggregatedSpend, PagesError>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Sync,
    Clock: Fn() -> DateTime<Utc> + Sync,
{
    let api_key_metadata = page_key_metadata(repository, records).await?;
    let owned_records = records.to_vec();
    let owned_entity_metadata = entity_metadata.cloned();
    let owned_entity_id_field = entity_id_field.map(str::to_owned);
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || {
        aggregate_spend_records_sync(
            &owned_records,
            &api_key_metadata,
            owned_entity_id_field.as_deref(),
            owned_entity_metadata.as_ref(),
        )
    });
    match tasks.join_next().await {
        Some(Ok(aggregated)) => aggregated,
        Some(Err(source)) => Err(PagesError::Task { source }),
        None => Err(PagesError::TaskMissing),
    }
}

async fn page_key_metadata<Database, Reads, Clock>(
    repository: &DailyActivityRepository<Database, Reads, Clock>,
    records: &[DailySpendRecord],
) -> Result<BTreeMap<String, KeyMetadataRow>, PagesError>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Sync,
    Clock: Fn() -> DateTime<Utc> + Sync,
{
    let api_keys: BTreeSet<String> = records
        .iter()
        .filter(|record| !record.api_key.is_empty() && record.api_key != PTU_SENTINEL_API_KEY)
        .map(|record| record.api_key.clone())
        .collect();
    if api_keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let dates: BTreeSet<String> = records.iter().map(|record| record.date.clone()).collect();
    let metadata = repository
        .key_metadata(&api_keys, spend_logs_window(&dates))
        .await?;
    Ok(metadata)
}

#[must_use]
pub fn aggregate_spend_records_sync(
    records: &[DailySpendRecord],
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
    entity_id_field: Option<&str>,
    entity_metadata: Option<&EntityMetadata>,
) -> Result<AggregatedSpend, PagesError> {
    let no_model_metadata = BTreeMap::new();
    let context = BreakdownContext {
        model_metadata: &no_model_metadata,
        provider_metadata: &no_model_metadata,
        api_key_metadata,
        entity_id_field,
        entity_metadata,
    };
    let mut totals = SpendMetrics::default();
    let mut grouped: IndexMap<String, DayGroup> = IndexMap::new();
    for record in records {
        let group = grouped
            .entry(record.date.clone())
            .or_insert_with(|| DayGroup {
                metrics: SpendMetrics::default(),
                breakdown: BreakdownMetrics::default(),
            });
        update_metrics(&mut group.metrics, record)?;
        update_breakdown_metrics(&mut group.breakdown, record, &context)?;
        update_metrics(&mut totals, record)?;
    }
    Ok(AggregatedSpend {
        results: dated_results(grouped)?,
        totals,
    })
}

fn dated_results(grouped: IndexMap<String, DayGroup>) -> Result<Vec<DailySpendData>, PagesError> {
    let mut results = Vec::with_capacity(grouped.len());
    for (raw_date, group) in grouped {
        let date = NaiveDate::parse_from_str(&raw_date, "%Y-%m-%d").map_err(|source| {
            PagesError::Date {
                value: raw_date,
                source,
            }
        })?;
        results.push(DailySpendData {
            date,
            metrics: group.metrics,
            breakdown: group.breakdown,
        });
    }
    results.sort_by_key(|day| Reverse(day.date));
    Ok(results)
}
