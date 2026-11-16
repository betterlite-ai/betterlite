use betterlite_analytics::daily_metrics::{
    BreakdownMetrics, KeyMetadata, KeyMetricWithMetadata, MetricWithMetadata, SpendMetrics,
};
use betterlite_db::daily_activity_contracts::KeyMetadataRow;
use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use indexmap::IndexMap;
use std::collections::BTreeMap;

use super::error::PagesError;
use super::metadata::{EntityMetadata, MetadataFields};
use super::record::DailySpendRecord;

macro_rules! accumulate {
    ($target:expr, $value:expr) => {
        $target = $target.checked_add($value).ok_or(PagesError::Overflow)?
    };
}

pub struct BreakdownContext<'text> {
    pub model_metadata: &'text BTreeMap<String, MetadataFields>,
    pub provider_metadata: &'text BTreeMap<String, MetadataFields>,
    pub api_key_metadata: &'text BTreeMap<String, KeyMetadataRow>,
    pub entity_id_field: Option<&'text str>,
    pub entity_metadata: Option<&'text EntityMetadata>,
}

pub fn update_breakdown_metrics(
    breakdown: &mut BreakdownMetrics,
    record: &DailySpendRecord,
    context: &BreakdownContext<'_>,
) -> Result<(), PagesError> {
    let is_ptu_sentinel = record.api_key == PTU_SENTINEL_API_KEY;
    let keys = (!is_ptu_sentinel).then_some(context.api_key_metadata);
    update_model_buckets(breakdown, record, context, is_ptu_sentinel)?;
    update_mcp_and_endpoints(breakdown, record, context)?;
    if !is_ptu_sentinel {
        let provider = record.custom_llm_provider.as_deref().unwrap_or("unknown");
        update_dimension(
            &mut breakdown.providers,
            Some(provider),
            context.provider_metadata,
            record,
            keys,
        )?;
        update_key_breakdown(&mut breakdown.api_keys, context.api_key_metadata, record)?;
    }
    update_entities(breakdown, record, context, keys)?;
    Ok(())
}

fn update_model_buckets(
    breakdown: &mut BreakdownMetrics,
    record: &DailySpendRecord,
    context: &BreakdownContext<'_>,
    is_ptu_sentinel: bool,
) -> Result<(), PagesError> {
    let keys = (!is_ptu_sentinel).then_some(context.api_key_metadata);
    let model_key = if is_ptu_sentinel {
        record.model_group.as_deref().or(record.model.as_deref())
    } else {
        record.model.as_deref()
    };
    update_dimension(
        &mut breakdown.models,
        model_key,
        context.model_metadata,
        record,
        keys,
    )?;
    let group_key = record.model_group.as_deref().or(record.model.as_deref());
    update_dimension(
        &mut breakdown.model_groups,
        group_key,
        context.model_metadata,
        record,
        keys,
    )
}

fn update_mcp_and_endpoints(
    breakdown: &mut BreakdownMetrics,
    record: &DailySpendRecord,
    context: &BreakdownContext<'_>,
) -> Result<(), PagesError> {
    let no_metadata = BTreeMap::new();
    // sentinels keep their key row on these two dimensions, unlike the model, provider and key buckets
    let keys = Some(context.api_key_metadata);
    update_dimension(
        &mut breakdown.mcp_servers,
        record.mcp_namespaced_tool_name.as_deref(),
        &no_metadata,
        record,
        keys,
    )?;
    update_dimension(
        &mut breakdown.endpoints,
        record.endpoint.as_deref(),
        &no_metadata,
        record,
        keys,
    )
}

fn update_entities(
    breakdown: &mut BreakdownMetrics,
    record: &DailySpendRecord,
    context: &BreakdownContext<'_>,
    keys: Option<&BTreeMap<String, KeyMetadataRow>>,
) -> Result<(), PagesError> {
    if context.entity_id_field.is_none() {
        return Ok(());
    }
    let no_metadata = BTreeMap::new();
    let entity = record.entity_id.as_deref().unwrap_or("Unassigned");
    update_dimension(
        &mut breakdown.entities,
        Some(entity),
        context.entity_metadata.unwrap_or(&no_metadata),
        record,
        keys,
    )
}

fn update_dimension(
    dimension: &mut IndexMap<String, MetricWithMetadata>,
    key: Option<&str>,
    metadata_fields: &BTreeMap<String, MetadataFields>,
    record: &DailySpendRecord,
    api_key_metadata: Option<&BTreeMap<String, KeyMetadataRow>>,
) -> Result<(), PagesError> {
    let Some(key) = key else {
        return Ok(());
    };
    let bucket = dimension
        .entry(key.to_owned())
        .or_insert_with(|| MetricWithMetadata {
            metrics: SpendMetrics::default(),
            metadata: metadata_fields.get(key).cloned().unwrap_or_default(),
            api_key_breakdown: IndexMap::new(),
        });
    update_metrics(&mut bucket.metrics, record)?;
    api_key_metadata.map_or(Ok(()), |api_key_metadata| {
        update_key_breakdown(&mut bucket.api_key_breakdown, api_key_metadata, record)
    })
}

fn update_key_breakdown(
    keys: &mut IndexMap<String, KeyMetricWithMetadata>,
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
    record: &DailySpendRecord,
) -> Result<(), PagesError> {
    let bucket = keys
        .entry(record.api_key.clone())
        .or_insert_with(|| key_metric(api_key_metadata, &record.api_key));
    update_metrics(&mut bucket.metrics, record)
}

fn key_metric(
    api_key_metadata: &BTreeMap<String, KeyMetadataRow>,
    api_key: &str,
) -> KeyMetricWithMetadata {
    let metadata = match api_key_metadata.get(api_key) {
        Some(row) => KeyMetadata {
            key_alias: row.key_alias.clone(),
            team_id: row.team_id.clone(),
            user_id: row.user_id.clone(),
            user_email: row.user_email.clone(),
            key_exists: Some(row.key_exists),
        },
        None => KeyMetadata {
            key_alias: None,
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: Some(false),
        },
    };
    KeyMetricWithMetadata {
        metrics: SpendMetrics::default(),
        metadata,
    }
}

pub(super) fn update_metrics(
    existing: &mut SpendMetrics,
    record: &DailySpendRecord,
) -> Result<(), PagesError> {
    accumulate!(existing.spend, record.spend);
    accumulate!(existing.flat_cost, record.flat_cost);
    accumulate!(existing.prompt_tokens, record.prompt_tokens);
    accumulate!(existing.completion_tokens, record.completion_tokens);
    accumulate!(
        existing.cache_read_input_tokens,
        record.cache_read_input_tokens
    );
    accumulate!(
        existing.cache_creation_input_tokens,
        record.cache_creation_input_tokens
    );
    accumulate!(
        existing.compression_saved_tokens,
        record.compression_saved_tokens
    );
    accumulate!(
        existing.compression_savings_spend,
        record.compression_savings_spend
    );
    accumulate!(
        existing.prompt_caching_savings_spend,
        record.prompt_caching_savings_spend
    );
    accumulate!(
        existing.gateway_caching_savings,
        record.gateway_caching_savings
    );
    accumulate!(
        existing.autorouter_savings_spend,
        record.autorouter_savings_spend
    );
    accumulate!(existing.api_requests, record.api_requests);
    accumulate!(existing.successful_requests, record.successful_requests);
    accumulate!(existing.failed_requests, record.failed_requests);
    accumulate!(
        existing.total_response_time_ms,
        record.total_response_time_ms
    );
    accumulate!(existing.timed_requests, record.timed_requests);
    let added_tokens = record
        .prompt_tokens
        .checked_add(record.completion_tokens)
        .ok_or(PagesError::Overflow)?;
    accumulate!(existing.total_tokens, added_tokens);
    Ok(())
}

#[path = "tests/breakdown.rs"]
mod tests;
