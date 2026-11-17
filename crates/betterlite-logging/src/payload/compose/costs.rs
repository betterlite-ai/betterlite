use betterlite_models::cost_breakdown::{CostBreakdown, cost_breakdown_with_guardrail};
use betterlite_models::decisions::Json;
use betterlite_models::guardrail_pricing::entries::{
    GuardrailInformation as PricedGuardrailInformation, guardrail_information_cost,
};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::payload::records::{DecimalValue, StandardLoggingHiddenParams};
use crate::fields::response::{ExtractedResponse, HiddenParams};
use crate::fields::status::LoggingStatus;
use crate::fields::timestamps::with_fraction;
use crate::logging_metadata::headers::get_hidden_params;
use crate::logging_metadata::params::MetadataMap;
use crate::payload::compose::identifiers::Identifiers;
use crate::payload::compose::inputs::optional_text;
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::deps::{ModelCostRequest, SavingsRequest};
use crate::payload::error::PayloadError;

pub(crate) const OVERHEAD_TIMING_KEY: &str = "litellm_overhead_time_ms";

pub(crate) struct Costs {
    pub response_cost: Decimal,
    pub cost_breakdown: Option<CostBreakdown>,
    pub autorouter_savings: Option<Decimal>,
    pub autorouter_savings_estimate: Option<IndexMap<String, Json>>,
    pub baseline_observation: Option<String>,
    pub saved_cache_cost: Decimal,
    pub model_map_information: IndexMap<String, Json>,
    pub raw_response_cost: Option<Decimal>,
}

pub(crate) struct CostsInput<'input> {
    pub request: &'input PayloadRequest<'input>,
    pub metadata: &'input MetadataMap,
    pub usage: &'input Json,
    pub identifiers: &'input Identifiers,
    pub model: &'input str,
}

impl Costs {
    pub(crate) fn read(
        input: &CostsInput<'_>,
        composition: &PayloadComposition<'_>,
    ) -> Result<Self, PayloadError> {
        let raw_response_cost =
            response_cost(input.request.state.model_call_details.get("response_cost"))?;
        let guardrail_cost =
            guardrail_cost(input.metadata.get("standard_logging_guardrail_information"))?;
        let response_cost = with_fraction(
            raw_response_cost
                .unwrap_or(Decimal::ZERO)
                .checked_add(guardrail_cost)
                .ok_or(PayloadError::CostOverflow)?,
        );
        let cost_breakdown = cost_breakdown_with_guardrail(
            input.request.state.cost_breakdown.as_ref(),
            guardrail_cost,
        )
        .map_err(|source| PayloadError::CostBreakdown { source })?;
        let baseline_observation = composition.sources.autorouter.baseline_observation();
        let savings = savings(input, cost_breakdown.as_ref(), composition);
        Ok(Self {
            response_cost,
            autorouter_savings_estimate: savings_estimate(
                input.metadata,
                savings.as_ref(),
                baseline_observation.as_ref(),
            ),
            autorouter_savings: savings,
            cost_breakdown,
            baseline_observation,
            saved_cache_cost: saved_cache_cost(input, composition),
            model_map_information: model_map_information(input, composition)?,
            raw_response_cost,
        })
    }
}

fn response_cost(value: Option<&Json>) -> Result<Option<Decimal>, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(|source| PayloadError::ResponseCost { source }),
        Some(_) => Err(PayloadError::ResponseCostType),
    }
}

fn guardrail_cost(value: Option<&Json>) -> Result<Decimal, PayloadError> {
    let Some(stored) = value else {
        return Ok(Decimal::ZERO);
    };
    let encoded = serde_json::to_vec(stored).map_err(|source| PayloadError::Json { source })?;
    let information: PricedGuardrailInformation =
        serde_json::from_slice(&encoded).map_err(|source| PayloadError::GuardrailInformation {
            field: "standard_logging_guardrail_information",
            source,
        })?;
    guardrail_information_cost(&information)
        .map_err(|source| PayloadError::GuardrailCost { source })
}

fn savings(
    input: &CostsInput<'_>,
    cost_breakdown: Option<&CostBreakdown>,
    composition: &PayloadComposition<'_>,
) -> Option<Decimal> {
    let cache_hit = input.identifiers.cache_hit == Some(true);
    if input.request.status != LoggingStatus::Success
        || cache_hit
        || composition.sources.autorouter.baseline_context_active()
    {
        return None;
    }
    composition
        .sources
        .autorouter
        .savings(&SavingsRequest {
            metadata: input.metadata,
            model: input.model,
            custom_llm_provider: input.request.state.custom_llm_provider.as_deref(),
            model_id: &input.identifiers.model_id,
            usage_object: input.usage,
            cost_breakdown,
        })
        .map(with_fraction)
}

fn savings_estimate(
    metadata: &MetadataMap,
    savings: Option<&Decimal>,
    baseline_observation: Option<&String>,
) -> Option<IndexMap<String, Json>> {
    if baseline_observation.is_some() {
        return Some(estimate(3, "unknown", "pending_projection"));
    }
    let routing_decision = metadata
        .get("routing_decision")
        .is_some_and(|value| !matches!(value, Json::Null));
    if !routing_decision {
        return None;
    }
    Some(match savings {
        Some(_) => estimate(1, "estimated", "uncached_usage"),
        None => estimate(1, "unknown", "baseline_unavailable"),
    })
}

fn estimate(version: i64, status: &str, reason: &str) -> IndexMap<String, Json> {
    IndexMap::from([
        ("version".to_owned(), Json::Number(Number::from(version))),
        ("status".to_owned(), Json::Text(status.to_owned())),
        ("reason".to_owned(), Json::Text(reason.to_owned())),
    ])
}

fn saved_cache_cost(input: &CostsInput<'_>, composition: &PayloadComposition<'_>) -> Decimal {
    if input.identifiers.cache_hit != Some(true) {
        return with_fraction(Decimal::ZERO);
    }
    with_fraction(
        composition
            .sources
            .saved_cache_cost
            .saved_cache_cost(input.request.init_response_obj)
            .unwrap_or(Decimal::ZERO),
    )
}

fn model_map_information(
    input: &CostsInput<'_>,
    composition: &PayloadComposition<'_>,
) -> Result<IndexMap<String, Json>, PayloadError> {
    let api_base = optional_text(
        input.request.state.litellm_params.get("api_base"),
        "litellm_params.api_base",
    )?;
    Ok(composition
        .sources
        .model_cost
        .model_map_information(&ModelCostRequest {
            model_call_details: &input.request.state.model_call_details,
            init_response_obj: input.request.init_response_obj,
            api_base: api_base.as_deref(),
            custom_llm_provider: input.request.state.custom_llm_provider.as_deref(),
        }))
}

pub(crate) fn hidden_params(
    request: &PayloadRequest<'_>,
    extracted: &ExtractedResponse,
    raw_response_cost: Option<Decimal>,
) -> Result<StandardLoggingHiddenParams, PayloadError> {
    let mut clean = match extracted.hidden_params.as_ref() {
        Some(HiddenParams::FromObject(fields)) => {
            get_hidden_params(Some(&Json::Object(fields.clone())))?
        }
        Some(HiddenParams::FromException(stored)) => stored.as_ref().clone(),
        None => StandardLoggingHiddenParams::default(),
    };
    if clean.response_cost.is_none()
        && let Some(raw) = raw_response_cost
    {
        clean.response_cost = Some(decimal_json(raw)?);
    }
    if clean.litellm_overhead_time_ms.is_none() && request.status == LoggingStatus::Success {
        // the metric is an f64; its shortest round-trip text is the decimal Python's json would print
        clean.litellm_overhead_time_ms = request
            .state
            .response_timing_metrics
            .get(OVERHEAD_TIMING_KEY)
            .and_then(|value| parse_decimal_lexeme(&value.to_string()).ok())
            .map(|value| DecimalValue(with_fraction(value)));
    }
    Ok(clean)
}

fn decimal_json(value: Decimal) -> Result<Json, PayloadError> {
    let text = with_fraction(value).to_string();
    Number::from_str(&text)
        .map(Json::Number)
        .map_err(|source| PayloadError::Json { source })
}
