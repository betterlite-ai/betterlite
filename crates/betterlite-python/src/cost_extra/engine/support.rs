use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_core::cost::realtime::{Event, realtime_stream, responses_ws};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageSumError, usage_sum};
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use rust_decimal::Decimal;

use crate::cost_extra::convert::UsageSource;
use crate::cost_extra::convert::usage::UsageWire;
use crate::cost_extra::convert::{CollectUsageRequest, CombineUsageRequest, PartitionUsageRequest};
use crate::cost_extra::error::RerankFailure;

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
    let usages = request
        .usages
        .iter()
        .map(UsageWire::usage)
        .collect::<Vec<Usage>>();
    usage_sum(&usages)
}

// The two collect_usage methods differ only in the event types they bill.
pub(crate) fn collect_usage(
    request: &CollectUsageRequest,
) -> Result<Vec<Usage>, RealtimeCostError> {
    match request.source {
        UsageSource::Realtime => realtime_stream::collect_usage(&request.results),
        UsageSource::ResponsesWs => responses_ws::collect_usage(&request.results),
    }
}

pub(crate) fn partition_usage(
    request: &PartitionUsageRequest,
) -> Result<Vec<(Option<String>, Vec<Event>)>, RealtimeCostError> {
    Ok(responses_ws::partition_by_service_tier(&request.results)?
        .into_iter()
        .collect())
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
    let (key, units) = match provider {
        JINA_PROVIDER | VOYAGE_PROVIDER => (TOKEN_RATE, units.total_tokens.flatten()),
        _ => (QUERY_RATE, units.search_units.flatten()),
    };
    let (Some(rate), Some(count)) = (numeric(record, key)?, units) else {
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
pub(crate) fn prompt_extras(details: Option<&PromptTokensDetails>) -> PromptExtras {
    let Some(details) = details else {
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
