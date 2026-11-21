use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::budget_gauges::gauge::label_pairs;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, prometheus_label_factory,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::RemainingModelGroupUsage;
use super::error::PrometheusRateLimitError;
use super::gauge::RateLimitGauges;
use super::metric::RateLimitMetric;

// the provider headers are stored with underscores, while the router reports the remaining counts under hyphenated keys
const PRESENT_TOKENS: &str = "x_ratelimit_remaining_tokens";
const PRESENT_REQUESTS: &str = "x_ratelimit_remaining_requests";
const REMAINING_TOKENS: &str = "x-ratelimit-remaining-tokens";
const REMAINING_REQUESTS: &str = "x-ratelimit-remaining-requests";

pub struct RouterRemainingInputs<'input> {
    pub payload: &'input StandardLoggingPayload,
    pub values: &'input UserApiKeyLabelValues,
    pub context: Option<&'input PrometheusLabelFactoryContext>,
    pub settings: &'input PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
    pub usage: Option<&'input dyn RemainingModelGroupUsage>,
}

// ast-grep-ignore: no-long-names
pub async fn async_set_router_remaining_metrics<G>(gauges: &G, inputs: RouterRemainingInputs<'_>)
where
    G: RateLimitGauges + Sync,
{
    let already_have_tokens = has_header(inputs.payload, PRESENT_TOKENS);
    let already_have_requests = has_header(inputs.payload, PRESENT_REQUESTS);
    if already_have_tokens && already_have_requests {
        return;
    }
    let Some(remaining_usage) = fetch_remaining_usage(inputs.payload, inputs.usage).await else {
        return;
    };
    let source = match inputs.context {
        Some(context) => LabelSource::Context(context),
        None => LabelSource::Values {
            settings: inputs.settings,
            end_user_settings: inputs.end_user_settings,
        },
    };
    for (already_have, metric, header) in [
        (
            already_have_tokens,
            RateLimitMetric::RemainingTokens,
            REMAINING_TOKENS,
        ),
        (
            already_have_requests,
            RateLimitMetric::RemainingRequests,
            REMAINING_REQUESTS,
        ),
    ] {
        if already_have {
            continue;
        }
        let Some(value) = remaining_usage.get(header) else {
            continue;
        };
        if let Err(error) = set_remaining(gauges, metric, *value, inputs.values, source) {
            tracing::error!("Prometheus: the remaining rate limit gauge could not be set: {error}");
            return;
        }
    }
}

// LiteLLM reads the router through a proxy-module global; here the port is a parameter, and its absence is a missing llm_router
async fn fetch_remaining_usage(
    payload: &StandardLoggingPayload,
    usage: Option<&dyn RemainingModelGroupUsage>,
) -> Option<BTreeMap<String, i64>> {
    let model_group = payload
        .model_group
        .as_deref()
        .filter(|group| !group.is_empty())?;
    let reader = usage?;
    match reader.get_remaining_model_group_usage(model_group).await {
        Ok(remaining_usage) => (!remaining_usage.is_empty()).then_some(remaining_usage),
        Err(error) => {
            tracing::error!("Prometheus: the remaining usage for {model_group} failed: {error}");
            None
        }
    }
}

fn set_remaining<G>(
    gauges: &G,
    metric: RateLimitMetric,
    value: i64,
    values: &UserApiKeyLabelValues,
    source: LabelSource<'_>,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    let projected = prometheus_label_factory(gauges.supported_labels(metric), values, source)?;
    let pairs = label_pairs(&projected);
    gauges
        .gauge(metric)
        .labels(&pairs)
        .set(MetricValue::Finite(Decimal::from(value)));
    Ok(())
}

// LiteLLM counts a header as present unless its value is None, and a null reaches it as None
fn has_header(payload: &StandardLoggingPayload, name: &str) -> bool {
    payload
        .hidden_params
        .additional_headers
        .as_ref()
        .and_then(|headers| headers.get(name))
        .is_some_and(|value| !matches!(value, Json::Null))
}
