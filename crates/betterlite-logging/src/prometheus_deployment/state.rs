use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::metric::{DeploymentMetric, DeploymentState};
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::DeploymentLabels;
use super::values::push_text;

pub fn set_litellm_deployment_state<G: DeploymentMetrics>(
    metrics: &G,
    state: DeploymentState,
    labels: DeploymentLabels<'_>,
) -> DeploymentResult<G> {
    let mut entries = Vec::new();
    push_text(
        &mut entries,
        "litellm_model_name",
        Some(labels.litellm_model_name),
    );
    push_text(&mut entries, "model_id", labels.model_id);
    push_text(&mut entries, "api_base", labels.api_base);
    push_text(&mut entries, "api_provider", Some(labels.api_provider));
    let values = UserApiKeyLabelValues::from_entries(&entries);
    metrics.set_gauge(
        DeploymentMetric::DeploymentState,
        &values,
        metrics.label_source(None),
        Decimal::from(state.number()),
    )
}

// ast-grep-ignore: no-one-line-functions - the three named setters are LiteLLM's entry points
pub fn set_deployment_healthy<G: DeploymentMetrics>(
    metrics: &G,
    labels: DeploymentLabels<'_>,
) -> DeploymentResult<G> {
    set_litellm_deployment_state(metrics, DeploymentState::Healthy, labels)
}

// ast-grep-ignore: no-one-line-functions - the three named setters are LiteLLM's entry points
pub fn set_deployment_partial_outage<G: DeploymentMetrics>(
    metrics: &G,
    labels: DeploymentLabels<'_>,
) -> DeploymentResult<G> {
    set_litellm_deployment_state(metrics, DeploymentState::PartialOutage, labels)
}

// ast-grep-ignore: no-one-line-functions - the three named setters are LiteLLM's entry points
pub fn set_deployment_complete_outage<G: DeploymentMetrics>(
    metrics: &G,
    labels: DeploymentLabels<'_>,
) -> DeploymentResult<G> {
    set_litellm_deployment_state(metrics, DeploymentState::CompleteOutage, labels)
}
