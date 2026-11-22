use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use crate::prometheus::failure::text_field;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::metric::DeploymentMetric;
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::{DeploymentLabels, FailureRequest};
use super::state::set_deployment_partial_outage;
use super::values::{failure_exception_status, object_field, tags_from_json, truthy};

mod identity;

const PROXY_REJECTED_BEFORE_ROUTING_KEY: &str = "proxy_rejected_before_routing";

struct FailureFields {
    model_id: Option<String>,
    model_group: Option<String>,
    api_provider: Option<String>,
    selected: bool,
}

struct DeploymentFields {
    litellm_model_name: Option<String>,
    model_id: Option<String>,
    api_base: Option<String>,
    api_provider: Option<String>,
    requested_model: Option<String>,
    model_group: Option<String>,
}

// ast-grep-ignore: no-long-names
pub fn set_llm_deployment_failure_metrics<G: DeploymentMetrics>(
    metrics: &G,
    request: &FailureRequest<'_>,
) {
    if let Err(error) = record_failure(metrics, request) {
        tracing::debug!(
            "Prometheus Error: set_llm_deployment_failure_metrics. Exception occured - {error}"
        );
    }
}

fn record_failure<G: DeploymentMetrics>(
    metrics: &G,
    request: &FailureRequest<'_>,
) -> DeploymentResult<G> {
    let fields = failure_fields(request);
    if metrics.skip_invalid_key(None, None, request.exception) {
        return Ok(());
    }
    let values = failure_values(metrics, request, &fields);
    if fields.selected {
        let outage = DeploymentLabels {
            litellm_model_name: request.model.unwrap_or_default(),
            model_id: fields.model_id.as_deref(),
            api_base: request.payload.map(|payload| payload.api_base.as_str()),
            api_provider: fields.api_provider.as_deref().unwrap_or_default(),
        };
        set_deployment_partial_outage(metrics, outage)?;
    }
    let context = PrometheusLabelFactoryContext::new(
        &values,
        metrics.settings(),
        metrics.end_user_settings(),
    )?;
    if request.exception.is_some() {
        metrics.increment_counter(
            DeploymentMetric::FailureResponses,
            &values,
            LabelSource::Context(&context),
            request.now,
        )?;
    }
    metrics.increment_counter(
        DeploymentMetric::TotalRequests,
        &values,
        LabelSource::Context(&context),
        request.now,
    )
}

fn failure_fields(request: &FailureRequest<'_>) -> FailureFields {
    let payload = request.payload;
    let model_id = payload
        .and_then(|payload| payload.model_id.clone())
        .or_else(|| params_model_id(request.litellm_params));
    let model_group = payload
        .and_then(|payload| payload.model_group.clone())
        .or_else(|| params_model_group(request.litellm_params));
    let selected = model_id.as_deref().is_some_and(|id| !id.is_empty())
        && !truthy(
            request
                .litellm_params
                .get(PROXY_REJECTED_BEFORE_ROUTING_KEY),
        );
    FailureFields {
        model_id,
        model_group,
        api_provider: text_field(request.litellm_params, "custom_llm_provider").map(str::to_owned),
        selected,
    }
}

fn params_model_id(params: &JsonObject) -> Option<String> {
    let internal = object_field(params, "litellm_metadata")
        .and_then(|metadata| object_field(metadata, "model_info"))
        .filter(|info| !info.is_empty());
    let caller = object_field(params, "metadata")
        .and_then(|metadata| object_field(metadata, "model_info"))
        .filter(|info| !info.is_empty());
    text_field(internal.or(caller)?, "id").map(str::to_owned)
}

fn params_model_group(params: &JsonObject) -> Option<String> {
    let internal = object_field(params, "litellm_metadata")
        .and_then(|metadata| metadata.get("model_group"))
        .filter(|value| truthy(Some(*value)));
    let caller = object_field(params, "metadata")
        .and_then(|metadata| metadata.get("model_group"))
        .filter(|value| truthy(Some(*value)));
    match internal.or(caller)? {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn failure_values<G: DeploymentMetrics>(
    metrics: &G,
    request: &FailureRequest<'_>,
    fields: &FailureFields,
) -> UserApiKeyLabelValues {
    let identity = identity::failure_identity(metrics, request);
    let deployment = if fields.selected {
        selected_fields(request, fields)
    } else {
        rejected_fields(metrics, request, fields)
    };
    UserApiKeyLabelValues {
        litellm_model_name: deployment.litellm_model_name,
        model_id: deployment.model_id,
        api_base: deployment.api_base,
        api_provider: deployment.api_provider,
        requested_model: deployment.requested_model,
        model_group: deployment.model_group,
        exception_status: request.exception.map(failure_exception_status),
        exception_class: request
            .exception
            .map(|exception| metrics.exception_class(exception)),
        hashed_api_key: identity.hashed_api_key,
        api_key_alias: identity.api_key_alias,
        user_email: identity.user_email,
        team: identity.team,
        team_alias: identity.team_alias,
        client_ip: identity.client_ip,
        user_agent: identity.user_agent,
        tags: tags_from_json(
            request
                .payload
                .map_or(&[], |payload| payload.request_tags.as_slice()),
        ),
        ..UserApiKeyLabelValues::default()
    }
}

fn selected_fields(request: &FailureRequest<'_>, fields: &FailureFields) -> DeploymentFields {
    DeploymentFields {
        litellm_model_name: request.model.map(str::to_owned),
        model_id: fields.model_id.clone(),
        api_base: request.payload.map(|payload| payload.api_base.clone()),
        api_provider: fields.api_provider.clone(),
        requested_model: fields
            .model_group
            .clone()
            .filter(|group| !group.is_empty())
            .or_else(|| request.model.map(str::to_owned)),
        model_group: fields.model_group.clone(),
    }
}

fn rejected_fields<G: DeploymentMetrics>(
    metrics: &G,
    request: &FailureRequest<'_>,
    fields: &FailureFields,
) -> DeploymentFields {
    let source = request
        .model
        .filter(|name| !name.is_empty())
        .or(fields.model_group.as_deref())
        .map(|name| Json::Text(name.to_owned()));
    let group = fields
        .model_group
        .as_deref()
        .map(|group| Json::Text(group.to_owned()));
    DeploymentFields {
        litellm_model_name: Some(String::new()),
        model_id: Some(String::new()),
        api_base: Some(String::new()),
        api_provider: Some(String::new()),
        requested_model: Some(
            metrics
                .bounded_model_label(source.as_ref(), true, request.catalog)
                .unwrap_or_default(),
        ),
        model_group: metrics.bounded_model_label(group.as_ref(), true, request.catalog),
    }
}
