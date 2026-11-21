use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::budget_gauges::gauge::label_pairs;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelFactoryError, LabelSource, ProjectedLabels, prometheus_label_factory,
};
use crate::prometheus_latency::labels::LabelInputs;

use super::error::PrometheusRateLimitError;
use super::gauge::RateLimitGauges;
// ast-grep-ignore: no-long-names
use super::headers::get_int_from_v3_rate_limit_headers;
use super::metric::{RateLimitDescriptor, RateLimitMetric, RateLimitType};

// one descriptor's allowed and used gauge for one rate limit type
#[derive(Clone, Copy, Debug)]
pub struct RateLimitSeries {
    pub rate_limit_type: RateLimitType,
    pub allowed: RateLimitMetric,
    pub used: RateLimitMetric,
}

// LiteLLM writes the pairs in this order, and each pair in the requests-then-tokens order
const DESCRIPTOR_SERIES: [(RateLimitDescriptor, RateLimitMetric, RateLimitMetric); 2] = [
    (
        RateLimitDescriptor::ApiKey,
        RateLimitMetric::ApiKeyRateLimitAllowed,
        RateLimitMetric::ApiKeyRateLimitUsed,
    ),
    (
        RateLimitDescriptor::Team,
        RateLimitMetric::TeamRateLimitAllowed,
        RateLimitMetric::TeamRateLimitUsed,
    ),
];

const RATE_LIMIT_TYPES: [RateLimitType; 2] = [RateLimitType::Requests, RateLimitType::Tokens];

// ast-grep-ignore: no-long-names
pub fn set_key_and_team_rate_limit_metrics<G>(
    gauges: &G,
    payload: &StandardLoggingPayload,
    labels: LabelInputs<'_>,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    for (descriptor, allowed, used) in DESCRIPTOR_SERIES {
        for rate_limit_type in RATE_LIMIT_TYPES {
            // ast-grep-ignore: no-long-names
            set_rate_limit_allowed_and_used_gauges(
                gauges,
                payload,
                labels,
                descriptor,
                RateLimitSeries {
                    rate_limit_type,
                    allowed,
                    used,
                },
            )?;
        }
    }
    Ok(())
}

// ast-grep-ignore: no-long-names
pub fn set_rate_limit_allowed_and_used_gauges<G>(
    gauges: &G,
    payload: &StandardLoggingPayload,
    labels: LabelInputs<'_>,
    descriptor: RateLimitDescriptor,
    series: RateLimitSeries,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    let (limit, remaining) = rate_limit_headers(payload, descriptor, series.rate_limit_type);
    let mut labelled_values = labels.values.clone();
    labelled_values.rate_limit_type = Some(series.rate_limit_type.as_str().to_owned());
    let context = PrometheusLabelFactoryContext::new(
        &labelled_values,
        labels.settings,
        labels.end_user_settings,
    )
    .map_err(LabelFactoryError::from)?;
    let labelnames = gauges.supported_labels(series.allowed);
    let projected =
        prometheus_label_factory(labelnames, &labelled_values, LabelSource::Context(&context))?;
    let (Some(limit_value), Some(remaining_value)) = (limit, remaining) else {
        return remove_rate_limit_series(gauges, series, labelnames, &projected);
    };
    let used = Decimal::from(limit_value)
        .checked_sub(Decimal::from(remaining_value))
        .ok_or(PrometheusRateLimitError::Overflow)?;
    let pairs = label_pairs(&projected);
    gauges
        .gauge(series.allowed)
        .labels(&pairs)
        .set(MetricValue::Finite(Decimal::from(limit_value)));
    gauges
        .gauge(series.used)
        .labels(&pairs)
        .set(MetricValue::Finite(used));
    Ok(())
}

fn rate_limit_headers(
    payload: &StandardLoggingPayload,
    descriptor: RateLimitDescriptor,
    rate_limit_type: RateLimitType,
) -> (Option<i64>, Option<i64>) {
    let limit_name = format!(
        "x-ratelimit-{}-limit-{}",
        descriptor.as_str(),
        rate_limit_type.as_str()
    );
    let remaining_name = format!(
        "x-ratelimit-{}-remaining-{}",
        descriptor.as_str(),
        rate_limit_type.as_str()
    );
    // ast-grep-ignore: no-long-names
    let limit = get_int_from_v3_rate_limit_headers(payload, &limit_name);
    // ast-grep-ignore: no-long-names
    let remaining = get_int_from_v3_rate_limit_headers(payload, &remaining_name);
    (limit, remaining)
}

// LiteLLM passes the label values in the metric's labelnames order, not the label dict's own order
fn remove_rate_limit_series<G>(
    gauges: &G,
    series: RateLimitSeries,
    labelnames: &[String],
    projected: &ProjectedLabels,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    let label_values: Vec<Option<String>> = labelnames
        .iter()
        .map(|name| projected.get(name).cloned().flatten())
        .collect();
    gauges
        .tracker()
        .remove_series(&gauges.gauge(series.allowed), &label_values)?;
    gauges
        .tracker()
        .remove_series(&gauges.gauge(series.used), &label_values)?;
    Ok(())
}
