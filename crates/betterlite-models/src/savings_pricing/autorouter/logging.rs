use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::savings_evidence::dynamic::MappingOrScalar;
use crate::savings_evidence::snapshot::CostBreakdown;

use super::{
    AutoRouterDecision, AutoRouterProvenance, AutoRouterRequestInput,
    autorouter_savings_for_request,
};
use crate::savings_pricing::context::SavingsPricingContext;
use crate::savings_pricing::usage::RequestUsage;

// The logged metadata keys savings reads; another key is ignored.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LoggingMetadata {
    pub internal_call_origin: Option<String>,
    pub routing_decision: Option<MappingOrScalar<AutoRouterDecision>>,
}

pub struct LoggingPayloadInput<'a> {
    pub metadata: &'a LoggingMetadata,
    pub model: Option<&'a str>,
    pub custom_llm_provider: Option<&'a str>,
    pub model_id: Option<&'a str>,
    pub usage: Option<&'a RequestUsage>,
    pub cost_breakdown: Option<&'a CostBreakdown>,
    pub baseline_usage: Option<&'a RequestUsage>,
    pub baseline_provenance: Option<AutoRouterProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'a TimeZoneDatabase,
}

// savings_for_logging_payload: internal sub-calls carry no savings figure of their own.
pub async fn savings_for_logging_payload<C>(
    context: &C,
    input: &LoggingPayloadInput<'_>,
) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let origin = input.metadata.internal_call_origin.as_deref();
    if origin.is_some_and(|value| !value.is_empty()) {
        return None;
    }
    let decision = input
        .metadata
        .routing_decision
        .as_ref()
        .and_then(MappingOrScalar::as_mapping);
    autorouter_savings_for_request(
        context,
        &AutoRouterRequestInput {
            model: input.model,
            custom_llm_provider: input.custom_llm_provider,
            routing_decision: decision,
            usage: input.usage,
            model_id: input.model_id,
            cost_breakdown: input.cost_breakdown,
            baseline_usage: input.baseline_usage,
            baseline_provenance: input.baseline_provenance,
            moment: input.moment,
            database: input.database,
        },
    )
    .await
}
