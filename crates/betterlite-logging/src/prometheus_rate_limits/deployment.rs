use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::gauge::label_pairs;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, prometheus_label_factory,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::error::{PrometheusRateLimitError, RateLimitValueError};
use super::gauge::RateLimitGauges;
use super::headers::declared_integer;
use super::metric::RateLimitMetric;

pub struct DeploymentLimitInputs<'input> {
    pub model_info: &'input JsonObject,
    pub litellm_params: &'input JsonObject,
    pub litellm_model_name: Option<&'input str>,
    pub model_id: Option<&'input str>,
    pub api_base: Option<&'input str>,
    pub api_provider: Option<&'input str>,
    pub model_group: Option<&'input str>,
    pub settings: &'input PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
}

// ast-grep-ignore: no-long-names
pub fn set_deployment_tpm_rpm_limit_metrics<G>(
    gauges: &G,
    inputs: DeploymentLimitInputs<'_>,
) -> Result<(), PrometheusRateLimitError<<G::Gauge as RemovableMetric>::Error>>
where
    G: RateLimitGauges,
{
    let tpm = limit_value(inputs.model_info, inputs.litellm_params, "tpm")?;
    let rpm = limit_value(inputs.model_info, inputs.litellm_params, "rpm")?;
    let values = deployment_values(&inputs);
    let source = LabelSource::Values {
        settings: inputs.settings,
        end_user_settings: inputs.end_user_settings,
    };
    for (metric, amount) in [
        (RateLimitMetric::DeploymentTpmLimit, tpm),
        (RateLimitMetric::DeploymentRpmLimit, rpm),
    ] {
        let Some(amount) = amount else {
            continue;
        };
        let projected = prometheus_label_factory(gauges.supported_labels(metric), &values, source)?;
        let pairs = label_pairs(&projected);
        gauges
            .gauge(metric)
            .labels(&pairs)
            .set(MetricValue::Finite(Decimal::from(amount)));
    }
    Ok(())
}

// model_info is read first and litellm_params only when it has no value, so a broken fallback never overrides a configured limit
fn limit_value(
    model_info: &JsonObject,
    litellm_params: &JsonObject,
    field: &str,
) -> Result<Option<i64>, RateLimitValueError> {
    let model_info_field = format!("model_info.{field}");
    if let Some(amount) = declared_integer(model_info.get(field), &model_info_field)? {
        return Ok(Some(amount));
    }
    let params_field = format!("litellm_params.{field}");
    declared_integer(litellm_params.get(field), &params_field)
}

fn deployment_values(inputs: &DeploymentLimitInputs<'_>) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = [
        ("litellm_model_name", inputs.litellm_model_name),
        ("model_id", inputs.model_id),
        ("api_base", inputs.api_base),
        ("api_provider", inputs.api_provider),
        ("model_group", inputs.model_group),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|text| (name.to_owned(), LabelDumpValue::String(text.to_owned())))
    })
    .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}
