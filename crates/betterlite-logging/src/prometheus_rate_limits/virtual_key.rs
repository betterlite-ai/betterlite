use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::budget_gauges::gauge::label_pairs;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::combined_custom_metadata::{
    CombinedMetadataError, combined_custom_metadata,
};
use crate::prometheus_label_projection::custom_metadata_extraction::custom_labels_from_metadata;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelFactoryError, LabelSource, prometheus_label_factory,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::error::{PrometheusRateLimitError, RateLimitValueError};
use super::gauge::RateLimitGauges;
// ast-grep-ignore: no-long-names
use super::headers::{declared_integer, get_remaining_from_v3_rate_limit_headers};
use super::metric::{RateLimitMetric, RateLimitType};

// Python's f-string renders a missing model group as the text None, and the legacy metadata key keeps it
const MISSING_MODEL_GROUP: &str = "None";

#[derive(Clone, Copy)]
pub struct VirtualKeyRateLimitInputs<'input> {
    pub user_api_key: Option<&'input str>,
    pub user_api_key_alias: Option<&'input str>,
    pub model_group: Option<&'input str>,
    pub model_id: Option<&'input str>,
    pub metadata: &'input IndexMap<String, Json>,
    pub payload: Option<&'input StandardLoggingPayload>,
    pub settings: &'input PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
}

// ast-grep-ignore: no-long-names
pub fn set_virtual_key_rate_limit_metrics<G>(
    gauges: &G,
    inputs: VirtualKeyRateLimitInputs<'_>,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    let requests = remaining_for_model(&inputs, RateLimitType::Requests)?;
    let tokens = remaining_for_model(&inputs, RateLimitType::Tokens)?;
    let values = virtual_key_values(&inputs)?;
    let context =
        PrometheusLabelFactoryContext::new(&values, inputs.settings, inputs.end_user_settings)
            .map_err(LabelFactoryError::from)?;
    for (metric, amount) in [
        (RateLimitMetric::RemainingApiKeyRequests, requests),
        (RateLimitMetric::RemainingApiKeyTokens, tokens),
    ] {
        let projected = prometheus_label_factory(
            gauges.supported_labels(metric),
            &values,
            LabelSource::Context(&context),
        )?;
        let pairs = label_pairs(&projected);
        gauges
            .gauge(metric)
            .labels(&pairs)
            .set(MetricValue::Finite(Decimal::from(amount)));
    }
    Ok(())
}

// the legacy limiter writes the remaining count into the request metadata under a model-group-specific name
fn remaining_for_model(
    inputs: &VirtualKeyRateLimitInputs<'_>,
    rate_limit_type: RateLimitType,
) -> Result<i64, RateLimitValueError> {
    let group_text = inputs.model_group.unwrap_or(MISSING_MODEL_GROUP);
    let key = format!(
        "litellm-key-remaining-{}-{group_text}",
        rate_limit_type.as_str()
    );
    let field = format!("metadata.{key}");
    if let Some(amount) = declared_integer(inputs.metadata.get(&key), &field)? {
        return Ok(amount);
    }
    // ast-grep-ignore: no-long-names
    let inherited = get_remaining_from_v3_rate_limit_headers(inputs.payload, rate_limit_type);
    Ok(inherited.unwrap_or(i64::MAX))
}

fn virtual_key_values(
    inputs: &VirtualKeyRateLimitInputs<'_>,
) -> Result<UserApiKeyLabelValues, CombinedMetadataError> {
    let payload_metadata = inputs.payload.map(|payload| &payload.metadata);
    let combined = combined_custom_metadata(payload_metadata)?;
    let custom_metadata_labels =
        custom_labels_from_metadata(&combined, &inputs.settings.custom_metadata_labels);
    let entries: Vec<(String, LabelDumpValue)> = [
        ("hashed_api_key", inputs.user_api_key),
        ("api_key_alias", inputs.user_api_key_alias),
        ("model", inputs.model_group),
        ("model_id", inputs.model_id),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|text| (name.to_owned(), LabelDumpValue::String(text.to_owned())))
    })
    .collect();
    let mut values = UserApiKeyLabelValues::from_entries(&entries);
    values.custom_metadata_labels = custom_metadata_labels;
    Ok(values)
}
