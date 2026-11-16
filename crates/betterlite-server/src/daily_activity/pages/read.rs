use betterlite_analytics::canonical_date::parse_canonical_date_range;
use betterlite_analytics::daily_metrics::{DailySpendMetadata, DailySpendPage, SpendMetrics};
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{DailyActivityProxyReads, DailyActivityScope};
use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::daily_activity_repository::DailyActivityRepositoryError;
use betterlite_db::daily_activity_repository::database::DailyActivityDatabase;
use chrono::{DateTime, Utc};
use std::future::Future;
use std::num::NonZeroI64;
use std::pin::Pin;

use super::aggregate::aggregate_spend_records;
use super::error::PagesError;
use super::metadata::EntityMetadata;
use super::record::DailySpendRecord;
use crate::daily_activity::recovery::scope::{ScopeIds, ScopeRequest, daily_activity_scope};

pub type EntityMetadataFuture = Pin<Box<dyn Future<Output = EntityMetadata> + Send>>;

pub type EntityMetadataResolver = dyn Fn(&[DailySpendRecord]) -> EntityMetadataFuture + Send + Sync;

pub type MetadataMetricsCallback = dyn Fn(&[DailySpendRecord]) -> SpendMetrics + Send + Sync;

#[derive(Clone, Copy, Default)]
pub struct PageCallbacks<'call> {
    pub metadata_metrics: Option<&'call MetadataMetricsCallback>,
    pub resolve_entity_metadata: Option<&'call EntityMetadataResolver>,
}

pub struct DailyActivityPageRequest<'text> {
    pub table: DailyActivityTable,
    pub entity_id_field: &'text str,
    pub entity_id: Option<ScopeIds>,
    pub exclude_entity_ids: Vec<String>,
    pub start_date: Option<&'text str>,
    pub end_date: Option<&'text str>,
    pub model: Option<&'text str>,
    pub api_key: Option<ScopeIds>,
    pub page: i64,
    pub page_size: i64,
    pub timezone_offset_minutes: Option<i64>,
    pub include_current_utc_day: bool,
    pub entity_metadata: Option<&'text EntityMetadata>,
    pub callbacks: PageCallbacks<'text>,
}

struct PagePosition {
    page: i64,
    total_pages: i64,
    has_more: bool,
}

pub async fn get_daily_activity<Database, Reads, Clock>(
    repository: &DailyActivityRepository<Database, Reads, Clock>,
    request: &DailyActivityPageRequest<'_>,
) -> Result<DailySpendPage, PagesError>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Sync,
    Clock: Fn() -> DateTime<Utc> + Sync,
{
    let scope = page_scope(request)?;
    let Some(page_size) = NonZeroI64::new(request.page_size).filter(|size| size.get() > 0) else {
        return Err(PagesError::Repository(
            DailyActivityRepositoryError::Pagination,
        ));
    };
    let page_data = repository
        .daily_rows(&scope, request.page, page_size.get())
        .await?;
    let records: Vec<DailySpendRecord> = page_data
        .rows
        .into_iter()
        .map(DailySpendRecord::from)
        .collect();
    let entity_metadata = resolved_entity_metadata(request, &records).await;
    let aggregated = aggregate_spend_records(
        repository,
        &records,
        Some(request.entity_id_field),
        entity_metadata.as_ref(),
    )
    .await?;
    let totals = request
        .callbacks
        .metadata_metrics
        .map_or(aggregated.totals, |metadata_metrics| {
            metadata_metrics(&records)
        });
    let position = page_position(request.page, page_size, page_data.total_count);
    Ok(DailySpendPage {
        results: aggregated.results,
        metadata: page_metadata(&totals, &position),
    })
}

fn page_scope(request: &DailyActivityPageRequest<'_>) -> Result<DailyActivityScope, PagesError> {
    let range = parse_canonical_date_range(request.start_date, request.end_date)
        .map_err(PagesError::DateRange)?;
    let start_date = range.start.to_string();
    let end_date = range.end.to_string();
    let scope = daily_activity_scope(ScopeRequest {
        table: request.table,
        entity_id_field: request.entity_id_field,
        entity_id: request.entity_id.clone(),
        exclude_entity_ids: request.exclude_entity_ids.clone(),
        api_key: request.api_key.clone(),
        start_date: &start_date,
        end_date: &end_date,
        model: request.model,
        timezone_offset_minutes: request.timezone_offset_minutes,
        include_current_utc_day: request.include_current_utc_day,
    })?;
    Ok(scope)
}

async fn resolved_entity_metadata(
    request: &DailyActivityPageRequest<'_>,
    records: &[DailySpendRecord],
) -> Option<EntityMetadata> {
    let Some(resolve) = request.callbacks.resolve_entity_metadata else {
        return request.entity_metadata.cloned();
    };
    let mut resolved = request.entity_metadata.cloned().unwrap_or_default();
    resolved.extend(resolve(records).await);
    Some(resolved)
}

fn page_position(page: i64, page_size: NonZeroI64, total_count: i64) -> PagePosition {
    PagePosition {
        page,
        total_pages: total_count.div_ceil(page_size.get()),
        has_more: page
            .checked_mul(page_size.get())
            .is_some_and(|seen| seen < total_count),
    }
}

const fn page_metadata(totals: &SpendMetrics, position: &PagePosition) -> DailySpendMetadata {
    DailySpendMetadata {
        total_spend: totals.spend,
        total_flat_cost: totals.flat_cost,
        total_prompt_tokens: totals.prompt_tokens,
        total_completion_tokens: totals.completion_tokens,
        total_tokens: totals.total_tokens,
        total_api_requests: totals.api_requests,
        total_successful_requests: totals.successful_requests,
        total_failed_requests: totals.failed_requests,
        total_cache_read_input_tokens: totals.cache_read_input_tokens,
        total_cache_creation_tokens: totals.cache_creation_input_tokens,
        total_compression_saved_tokens: totals.compression_saved_tokens,
        total_compression_savings: totals.compression_savings_spend,
        total_prompt_caching_savings: totals.prompt_caching_savings_spend,
        total_gateway_caching_savings: totals.gateway_caching_savings,
        total_autorouter_savings_spend: totals.autorouter_savings_spend,
        total_response_time_ms: totals.total_response_time_ms,
        total_timed_requests: totals.timed_requests,
        page: position.page,
        total_pages: position.total_pages,
        has_more: position.has_more,
        api_key_limit: None,
        total_api_keys: None,
        entity_total_api_keys: None,
    }
}
