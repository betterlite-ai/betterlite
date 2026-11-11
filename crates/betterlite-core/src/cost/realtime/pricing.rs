use std::collections::BTreeMap;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::{
    GenericCostRequest, GenericCostResult, generic_cost_per_token,
};
use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::Event;
use super::error::RealtimeCostError;
use super::transcription::{self, TranscriptionCostRequest};
use crate::cost::context::storage::store_cost_breakdown;

const NON_TOKEN_RATE_FIELDS: [&str; 5] = [
    "cost_per_second",
    "input_cost_per_second",
    "output_cost_per_second",
    "input_cost_per_query",
    "tiered_pricing",
];

pub struct RealtimeCostRequest<'request> {
    pub results: &'request [Event],
    pub combined_usage: &'request Usage,
    pub custom_llm_provider: &'request str,
    pub litellm_model_name: &'request str,
    pub data_residency: Option<&'request str>,
    pub custom_pricing_model: Option<&'request str>,
    pub base_pricing_model: Option<&'request str>,
    pub moment: DateTime<Utc>,
}

struct SessionCosts {
    input: Decimal,
    output: Decimal,
    transcription: Decimal,
    total: Decimal,
}

// Ports handle_realtime_stream_cost_calculation: candidate token prices, the transcription charge and its breakdown.
pub fn stream_cost<Target>(
    request: &RealtimeCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
    target: Option<&mut Target>,
) -> Result<Decimal, RealtimeCostError>
where
    Target: CostBreakdownTarget,
{
    let names = candidate_names(request)?;
    let (input, output) = first_priced_costs(&names, request, lookup, database)?;
    let transcription = transcription_charge(request, lookup)?;
    let total = input
        .checked_add(output)
        .and_then(|subtotal| subtotal.checked_add(transcription))
        .ok_or(RealtimeCostError::Overflow)?;
    let costs = SessionCosts {
        input,
        output,
        transcription,
        total,
    };
    store_breakdown(target, &costs, request.data_residency);
    Ok(total)
}

fn transcription_charge(
    request: &RealtimeCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, RealtimeCostError> {
    if !request.results.iter().any(|event| {
        matches!(
            event.get("type"),
            Some(Json::Text(event_type))
                if event_type == transcription::TRANSCRIPTION_COMPLETED_EVENT
        )
    }) {
        return Ok(Decimal::ZERO);
    }
    transcription::cost(
        &TranscriptionCostRequest {
            results: request.results,
            custom_llm_provider: request.custom_llm_provider,
            litellm_model_name: request.litellm_model_name,
            custom_pricing_model: request.custom_pricing_model,
        },
        lookup,
    )
}

fn store_breakdown<Target>(
    target: Option<&mut Target>,
    costs: &SessionCosts,
    data_residency: Option<&str>,
) where
    Target: CostBreakdownTarget,
{
    let additional_costs = (costs.transcription > Decimal::ZERO)
        .then(|| BTreeMap::from([("transcription_cost".to_owned(), costs.transcription)]));
    store_cost_breakdown(
        target,
        &CostBreakdownInput {
            input_cost: costs.input,
            output_cost: costs.output,
            total_cost: costs.total,
            tool_usage_cost: Decimal::ZERO,
            additional_costs,
            original_cost: None,
            discount_percent: None,
            discount_amount: None,
            margin_percent: None,
            margin_fixed_amount: None,
            margin_total_amount: None,
            cache_read_cost: None,
            cache_creation_cost: None,
            reasoning_cost: None,
            service_tier: None,
            data_residency: data_residency.map(str::to_owned),
            vertex_location: None,
            billed_token_rates: None,
        },
    );
}

// Every session.created model in stream order, then the request's own model; None candidates are skipped.
fn candidate_names(
    request: &RealtimeCostRequest<'_>,
) -> Result<Vec<Option<String>>, RealtimeCostError> {
    let mut names = vec![
        request.custom_pricing_model.map(str::to_owned),
        request.base_pricing_model.map(str::to_owned),
    ];
    for event in request.results {
        let Some(kind) = event.get("type") else {
            return Err(RealtimeCostError::MissingField { field: "type" });
        };
        if !matches!(kind, Json::Text(event_type) if event_type == "session.created") {
            continue;
        }
        let Some(session_value) = event.get("session") else {
            return Err(RealtimeCostError::MissingField { field: "session" });
        };
        let Json::Object(session) = session_value else {
            return Err(RealtimeCostError::Shape {
                field: "session",
                expected: "an object",
            });
        };
        names.push(match session.get("model") {
            None | Some(Json::Null) => None,
            Some(Json::Text(model)) => Some(model.clone()),
            Some(_) => {
                return Err(RealtimeCostError::Shape {
                    field: "session.model",
                    expected: "a string or null",
                });
            }
        });
    }
    names.push(Some(request.litellm_model_name.to_owned()));
    Ok(names)
}

// Ports _first_priced_realtime_token_costs: a failed candidate is skipped and a declared free entry still wins.
fn first_priced_costs(
    names: &[Option<String>],
    request: &RealtimeCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<(Decimal, Decimal), RealtimeCostError> {
    for name in names.iter().flatten() {
        let Some(costs) = candidate_costs(name, request, lookup, database) else {
            continue;
        };
        let sum = costs
            .prompt_cost
            .checked_add(costs.completion_cost)
            .ok_or(RealtimeCostError::Overflow)?;
        if sum > Decimal::ZERO || declared_pricing(name, request.custom_llm_provider, lookup) {
            return Ok((costs.prompt_cost, costs.completion_cost));
        }
    }
    Ok((Decimal::ZERO, Decimal::ZERO))
}

fn candidate_costs(
    name: &str,
    request: &RealtimeCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Option<GenericCostResult> {
    let usage = CostUsage::from(request.combined_usage);
    let extras = prompt_extras(request.combined_usage.prompt_tokens_details.as_ref());
    generic_cost_per_token(
        &GenericCostRequest {
            model: name,
            custom_llm_provider: Some(request.custom_llm_provider),
            usage: &usage,
            prompt_extras: &extras,
            supplied_pricing: None,
            service_tier: None,
            data_residency: request.data_residency,
            vertex_location: None,
            moment: request.moment,
        },
        lookup,
        database,
    )
    .ok()
}

fn prompt_extras(source: Option<&PromptTokensDetails>) -> PromptExtras {
    let Some(details) = source else {
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

// Ports _cost_map_entry_declares_pricing: the resolved key, the model name and the provider-scoped name.
fn declared_pricing(model: &str, provider: &str, lookup: &dyn PriceLookup) -> bool {
    let resolved =
        lookup
            .effective_model_info(model, Some(provider))
            .and_then(|record| match record.get("key") {
                Some(Scalar::Text(key)) => lookup.raw_cost_map_entry(key),
                _ => None,
            });
    let direct = lookup.raw_cost_map_entry(model);
    let provider_scoped = lookup.raw_cost_map_entry(&format!("{provider}/{model}"));
    [resolved, direct, provider_scoped]
        .iter()
        .flatten()
        .any(entry_prices_anything)
}

// Ports _cost_map_entry_prices_anything: a nonnull seconds, query, tiered or token rate declares, zero included.
fn entry_prices_anything(entry: &PriceRecord) -> bool {
    entry.iter().any(|(field, value)| {
        let token_rate = field.contains("cost_per") && field.contains("token");
        !matches!(value, Scalar::Null)
            && (NON_TOKEN_RATE_FIELDS.contains(&field.as_str()) || token_rate)
    })
}
