use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::metric::{DeploymentMetric, DeploymentState};
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::DeploymentLabels;

pub fn set_litellm_deployment_state<G: DeploymentMetrics>(
    metrics: &G,
    state: DeploymentState,
    labels: DeploymentLabels<'_>,
) -> DeploymentResult<G> {
    let values = UserApiKeyLabelValues {
        litellm_model_name: Some(labels.litellm_model_name.to_owned()),
        model_id: labels.model_id.map(str::to_owned),
        api_base: labels.api_base.map(str::to_owned),
        api_provider: Some(labels.api_provider.to_owned()),
        ..UserApiKeyLabelValues::default()
    };
    let source = metrics.label_source(None);
    metrics.set_gauge(
        DeploymentMetric::DeploymentState,
        &values,
        source,
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
