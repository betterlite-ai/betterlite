use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_core::api::responses_usage::{
    ResponsesUsageError, UsageInput, normalize_and_project,
};
use betterlite_core::cost::realtime::{Event, error::RealtimeCostError, responses_ws};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::fields::usage::Usage as LoggingUsage;
use betterlite_logging::fields::usage::UsageError as LoggingUsageError;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageSumError, usage_sum};
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::cost_extra::convert::UsageSource;
use crate::cost_extra::convert::{CollectUsageRequest, CombineUsageRequest, PartitionUsageRequest};
use crate::cost_extra::error::RerankFailure;

mod tests;

const QUERY_RATE: &str = "input_cost_per_query";
const TOKEN_RATE: &str = "input_cost_per_token";
const JINA_PROVIDER: &str = "jina_ai";
const VOYAGE_PROVIDER: &str = "voyage";

// The breakdown a realtime session leaves for Python; LiteLLM's own logging object keeps it.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    pub(crate) breakdown: Option<CostBreakdown>,
}

impl CostBreakdownTarget for Capture {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.breakdown = Some(CostBreakdown {
            service_tier: input.service_tier.clone(),
            data_residency: input.data_residency.clone(),
            vertex_location: input.vertex_location.clone(),
            input_cost: Some(input.input_cost),
            cache_read_cost: input.cache_read_cost,
            cache_creation_cost: input.cache_creation_cost,
            output_cost: Some(input.output_cost),
            reasoning_cost: input.reasoning_cost,
            total_cost: Some(input.total_cost),
            tool_usage_cost: Some(input.tool_usage_cost),
            guardrail_cost: None,
            additional_costs: input
                .additional_costs
                .clone()
                .filter(|costs| !costs.is_empty()),
            original_cost: input.original_cost,
            discount_percent: input.discount_percent,
            discount_amount: input.discount_amount,
            margin_percent: input.margin_percent,
            margin_fixed_amount: input.margin_fixed_amount,
            margin_total_amount: input.margin_total_amount,
            extra: BTreeMap::new(),
        });
        Ok(())
    }
}

// combine_usage_objects: every counter and detail summed, an empty list giving LiteLLM's default usage.
pub(crate) fn combine_usage(request: &CombineUsageRequest) -> Result<Usage, UsageSumError> {
    usage_sum(
        &request
            .usages
            .iter()
            .map(Usage::from)
            .collect::<Vec<Usage>>(),
    )
}

// A collected event fails either in the session filter or in the full usage transformer after it.
#[derive(Debug, Error)]
pub(crate) enum CollectFailure {
    #[error(transparent)]
    Realtime(#[from] RealtimeCostError),
    #[error(transparent)]
    Responses(#[from] ResponsesUsageError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingUsageError),
}

// The two collect_usage methods differ only in the events they bill; each usage keeps every extension.
pub(crate) fn collect_usage(
    request: &CollectUsageRequest,
) -> Result<Vec<LoggingUsage>, CollectFailure> {
    let mappings: Vec<IndexMap<String, Json>> = match request.source {
        UsageSource::Realtime => realtime_usages(&request.results)?,
        UsageSource::ResponsesWs => responses_ws::billable_events(&request.results)?
            .into_iter()
            .map(|billable| billable.usage.clone())
            .collect(),
    };
    mappings.iter().map(usage_record).collect()
}

fn usage_record(mapping: &IndexMap<String, Json>) -> Result<LoggingUsage, CollectFailure> {
    let source = serde_json::to_value(Json::Object(mapping.clone()))?;
    let projected = normalize_and_project(UsageInput::Mapping(&source))?;
    let projected_mapping: IndexMap<String, Json> = serde_json::from_value(projected)?;
    usage_from_mapping(&projected_mapping).map_err(CollectFailure::from)
}

// collect_usage_from_realtime_stream_results: response.done events, each with its response object.
fn realtime_usages(results: &[Event]) -> Result<Vec<IndexMap<String, Json>>, RealtimeCostError> {
    let mut usages = Vec::new();
    for event in results {
        let Some(kind) = event.get("type") else {
            return Err(RealtimeCostError::MissingField { field: "type" });
        };
        if !matches!(kind, Json::Text(event_type) if event_type == "response.done") {
            continue;
        }
        let Some(response) = event.get("response") else {
            return Err(RealtimeCostError::MissingField { field: "response" });
        };
        let Json::Object(response_object) = response else {
            return Err(RealtimeCostError::Shape {
                field: "response",
                expected: "an object",
            });
        };
        match response_object.get("usage") {
            None | Some(Json::Null) => usages.push(IndexMap::new()),
            Some(Json::Object(usage)) => usages.push(usage.clone()),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(RealtimeCostError::Shape {
                    field: "usage",
                    expected: "an object or null",
                });
            }
        }
    }
    Ok(usages)
}

pub(crate) type ServiceTiers = Vec<(Option<String>, Vec<Event>)>;

pub(crate) fn partition_usage(
    request: &PartitionUsageRequest,
) -> Result<ServiceTiers, RealtimeCostError> {
    responses_ws::partition_by_service_tier(&request.results)
        .map(|tiers| tiers.into_iter().collect())
}

// Ports jina's and voyage's rerank override; every other provider bills the query units.
pub(crate) fn rerank_price(
    provider: &str,
    record: Option<&PriceRecord>,
    billed_units: Option<&RerankBilledUnits>,
) -> Result<(Decimal, Decimal), RerankFailure> {
    let Some(units) = billed_units else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let (key, counted) = match provider {
        JINA_PROVIDER | VOYAGE_PROVIDER => (TOKEN_RATE, units.total_tokens.flatten()),
        _ => (QUERY_RATE, units.search_units.flatten()),
    };
    let (Some(rate), Some(count)) = (numeric(record, key)?, counted) else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let cost = rate
        .checked_mul(Decimal::from(count))
        .ok_or(RerankFailure::Overflow)?;
    Ok((cost, Decimal::ZERO))
}

pub(crate) fn numeric(
    record: Option<&PriceRecord>,
    key: &str,
) -> Result<Option<Decimal>, RerankFailure> {
    match record.and_then(|found| found.get(key)) {
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(RerankFailure::Rate),
        Some(Scalar::Bool(flag)) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_)) | None => {
            Ok(None)
        }
    }
}

// The prompt details batch pricing reads besides the counters, spelled as batch_cost_calculator passes them.
pub(crate) fn prompt_extras(provided: Option<&PromptTokensDetails>) -> PromptExtras {
    let Some(details) = provided else {
        return PromptExtras::default();
    };
    PromptExtras {
        cache_write_tokens: details.cache_write_tokens,
        cache_creation_tokens: details.cache_creation_tokens,
        cache_creation_token_details: None,
        cached_tokens_details: details.cached_tokens_details.as_ref().map(|cached| {
            CachedTokenCounters {
                text_tokens: cached.text_tokens,
                audio_tokens: cached.audio_tokens,
                image_tokens: cached.image_tokens,
            }
        }),
        character_count: details.character_count,
        image_count: details.image_count,
        video_length_seconds: details.video_length_seconds,
        audio_length_seconds: details.audio_length_seconds,
        query_count: details.query_count,
    }
}
