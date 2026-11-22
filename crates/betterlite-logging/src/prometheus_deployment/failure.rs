use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};

use crate::prometheus::failure::text_field;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::metric::DeploymentMetric;
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::{DeploymentLabels, FailureRequest};
use super::state::set_deployment_partial_outage;
use super::values::{
    failure_exception_status, model_group_field, object_field, push_identity, push_text,
    tags_from_json,
};

mod identity;

const PROXY_REJECTED_KEY: &str = "proxy_rejected_before_routing";

struct FailureFields {
    model_id: Option<String>,
    model_group: Option<String>,
    api_provider: Option<String>,
    selected: bool,
}

// ast-grep-ignore: no-long-names
pub fn set_llm_deployment_failure_metrics<G>(metrics: &G, request: &FailureRequest<'_>)
where
    G: DeploymentMetrics,
{
    if let Err(error) = record_failure(metrics, request) {
        tracing::debug!(
            "Prometheus Error: set_llm_deployment_failure_metrics. Exception occured - {error}"
        );
    }
}

fn record_failure<G>(metrics: &G, request: &FailureRequest<'_>) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
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
    let rejected = matches!(
        request.litellm_params.get(PROXY_REJECTED_KEY),
        Some(Json::Bool(true))
    );
    FailureFields {
        selected: model_id.as_deref().is_some_and(|id| !id.is_empty()) && !rejected,
        model_id,
        model_group,
        api_provider: text_field(request.litellm_params, "custom_llm_provider").map(str::to_owned),
    }
}

// LiteLLM reads litellm_metadata first and the caller's metadata second, and a falsy one falls through
fn metadata_sources(params: &JsonObject) -> [Option<&JsonObject>; 2] {
    [
        object_field(params, "litellm_metadata"),
        object_field(params, "metadata"),
    ]
}

fn params_model_id(params: &JsonObject) -> Option<String> {
    let model_info = metadata_sources(params)
        .into_iter()
        .filter_map(|metadata| object_field(metadata?, "model_info"))
        .find(|info| !info.is_empty())?;
    text_field(model_info, "id").map(str::to_owned)
}

fn params_model_group(params: &JsonObject) -> Option<String> {
    metadata_sources(params)
        .into_iter()
        .find_map(|metadata| metadata?.get("model_group").and_then(model_group_field))
}

fn failure_values<G>(
    metrics: &G,
    request: &FailureRequest<'_>,
    fields: &FailureFields,
) -> UserApiKeyLabelValues
where
    G: DeploymentMetrics,
{
    let identity = identity::failure_identity(metrics, request);
    let mut entries: Vec<(String, LabelDumpValue)> = Vec::new();
    deployment_values(metrics, request, fields, &mut entries);
    let status = request.exception.map(failure_exception_status);
    push_text(&mut entries, "exception_status", status.as_deref());
    let class = request
        .exception
        .map(|exception| metrics.exception_class(exception));
    push_text(&mut entries, "exception_class", class.as_deref());
    push_identity(
        &mut entries,
        identity.hashed_api_key.as_deref(),
        identity.api_key_alias.as_deref(),
        identity.team.as_deref(),
        identity.team_alias.as_deref(),
    );
    push_text(&mut entries, "user_email", identity.user_email.as_deref());
    push_text(&mut entries, "client_ip", identity.client_ip.as_deref());
    push_text(&mut entries, "user_agent", identity.user_agent.as_deref());
    let tags = tags_from_json(
        request
            .payload
            .map_or(&[], |payload| payload.request_tags.as_slice()),
    );
    entries.push(("tags".to_owned(), LabelDumpValue::Strings(tags)));
    UserApiKeyLabelValues::from_entries(&entries)
}

fn deployment_values<G>(
    metrics: &G,
    request: &FailureRequest<'_>,
    fields: &FailureFields,
    entries: &mut Vec<(String, LabelDumpValue)>,
) where
    G: DeploymentMetrics,
{
    if fields.selected {
        push_text(entries, "litellm_model_name", request.model);
        push_text(entries, "model_id", fields.model_id.as_deref());
        push_text(
            entries,
            "api_base",
            request.payload.map(|payload| payload.api_base.as_str()),
        );
        push_text(entries, "api_provider", fields.api_provider.as_deref());
        let requested = fields
            .model_group
            .as_deref()
            .filter(|group| !group.is_empty());
        push_text(entries, "requested_model", requested.or(request.model));
        push_text(entries, "model_group", fields.model_group.as_deref());
    } else {
        rejected_values(metrics, request, fields, entries);
    }
}

// a LiteLLM-side reject has no deployment, so only the bounded caller names label the request
fn rejected_values<G>(
    metrics: &G,
    request: &FailureRequest<'_>,
    fields: &FailureFields,
    entries: &mut Vec<(String, LabelDumpValue)>,
) where
    G: DeploymentMetrics,
{
    for name in ["litellm_model_name", "model_id", "api_base", "api_provider"] {
        push_text(entries, name, Some(""));
    }
    let source = request
        .model
        .filter(|name| !name.is_empty())
        .or(fields.model_group.as_deref())
        .map(|name| Json::Text(name.to_owned()));
    let bounded = metrics.bounded_model_label(source.as_ref(), true, request.catalog);
    push_text(
        entries,
        "requested_model",
        Some(bounded.as_deref().unwrap_or_default()),
    );
    let group = fields
        .model_group
        .as_deref()
        .map(|name| Json::Text(name.to_owned()));
    let bounded_group = metrics.bounded_model_label(group.as_ref(), true, request.catalog);
    push_text(entries, "model_group", bounded_group.as_deref());
}
