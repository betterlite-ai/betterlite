use std::time::Instant;

use betterlite_models::call_helpers::metadata::{
    CallParams, InternalMetadata, litellm_metadata_from_params,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::text_field;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, prometheus_label_factory,
};
use crate::prometheus_latency::guardrail_overhead::{
    GuardrailRecords, OverheadObservation, set_guardrail_overhead_metric,
};
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_latency::latency_histograms::MetricHandle;
use crate::prometheus_rate_limits::deployment::DeploymentLimitInputs;
// ast-grep-ignore: no-long-names
use crate::prometheus_rate_limits::deployment::set_deployment_tpm_rpm_limit_metrics as set_deployment_limit_metrics;

use super::error::{HeaderValueError, PrometheusDeploymentError};
use super::metric::DeploymentMetric;
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::{DeploymentLabels, SuccessRequest};
use super::state::set_deployment_healthy;
use super::values::{decimal_text, object_field};

const REMAINING_REQUESTS_HEADER: &str = "x_ratelimit_remaining_requests";
const REMAINING_TOKENS_HEADER: &str = "x_ratelimit_remaining_tokens";
const MILLIS_PER_SECOND: Decimal = Decimal::ONE_THOUSAND;

struct SuccessFields {
    model_info: JsonObject,
    model_id: Option<String>,
    llm_provider: Option<String>,
}

// ast-grep-ignore: no-long-names
pub fn set_llm_deployment_success_metrics<G>(metrics: &G, request: &SuccessRequest<'_>)
where
    G: DeploymentMetrics,
{
    if let Err(error) = record_success(metrics, request) {
        tracing::error!(
            "Prometheus Error: set_llm_deployment_success_metrics. Exception occured - {error}"
        );
    }
}

fn record_success<G>(metrics: &G, request: &SuccessRequest<'_>) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    let Some(payload) = request.payload else {
        return Ok(());
    };
    if metrics.skip_invalid_key(None, None, request.exception) {
        return Ok(());
    }
    let fields = success_fields(request);
    record_limits(metrics, request, payload, &fields)?;
    record_overhead(metrics, request, payload)?;
    record_remaining(metrics, request, payload)?;
    let outage = DeploymentLabels {
        litellm_model_name: request.model.unwrap_or_default(),
        model_id: Some(fields.model_id.as_deref().unwrap_or_default()),
        api_base: Some(payload.api_base.as_str()),
        api_provider: fields.llm_provider.as_deref().unwrap_or_default(),
    };
    set_deployment_healthy(metrics, outage)?;
    let source = metrics.label_source(request.context);
    record_success_totals(metrics, request.values, source, request.now)?;
    record_latency(metrics, request, source)
}

fn success_fields(request: &SuccessRequest<'_>) -> SuccessFields {
    let call_params = CallParams {
        metadata: object_field(request.litellm_params, "metadata").cloned(),
        litellm_metadata: object_field(request.litellm_params, "litellm_metadata").map(|entries| {
            InternalMetadata {
                entries: entries.clone(),
            }
        }),
    };
    let metadata = litellm_metadata_from_params(Some(&call_params));
    let model_info = object_field(&metadata, "model_info")
        .cloned()
        .unwrap_or_default();
    SuccessFields {
        model_id: text_field(&model_info, "id").map(str::to_owned),
        llm_provider: text_field(request.litellm_params, "custom_llm_provider").map(str::to_owned),
        model_info,
    }
}

fn record_success_totals<G>(
    metrics: &G,
    values: &UserApiKeyLabelValues,
    source: LabelSource<'_>,
    now: Instant,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    for metric in [
        DeploymentMetric::SuccessResponses,
        DeploymentMetric::TotalRequests,
    ] {
        metrics.increment_counter(metric, values, source, now)?;
    }
    Ok(())
}

fn record_limits<G>(
    metrics: &G,
    request: &SuccessRequest<'_>,
    payload: &StandardLoggingPayload,
    fields: &SuccessFields,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    if fields.model_info.is_empty() && request.litellm_params.is_empty() {
        return Ok(());
    }
    set_deployment_limit_metrics(
        metrics,
        DeploymentLimitInputs {
            model_info: &fields.model_info,
            litellm_params: request.litellm_params,
            litellm_model_name: request.model,
            model_id: fields.model_id.as_deref(),
            api_base: Some(payload.api_base.as_str()),
            api_provider: fields.llm_provider.as_deref(),
            model_group: request.values.model_group.as_deref(),
            settings: metrics.settings(),
            end_user_settings: metrics.end_user_settings(),
        },
    )?;
    Ok(())
}

fn record_overhead<G>(
    metrics: &G,
    request: &SuccessRequest<'_>,
    payload: &StandardLoggingPayload,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    let overhead_ms = payload
        .hidden_params
        .litellm_overhead_time_ms
        .as_ref()
        .map(|value| value.0);
    record_overhead_latency(metrics, request, overhead_ms)?;
    let records = payload
        .guardrail_information
        .as_deref()
        .map_or(GuardrailRecords::Absent, GuardrailRecords::Multiple);
    let observation = OverheadObservation {
        overhead_ms,
        guardrail_records: records,
    };
    let inputs = LabelInputs {
        values: request.values,
        settings: metrics.settings(),
        end_user_settings: metrics.end_user_settings(),
        context: request.context,
    };
    let histogram = metrics.metric(DeploymentMetric::OverheadWithGuardrails);
    let handle = MetricHandle {
        metric: &histogram,
        supported_labels: metrics.labels_for(DeploymentMetric::OverheadWithGuardrails),
    };
    set_guardrail_overhead_metric(&handle, &observation, &inputs)?;
    Ok(())
}

fn record_overhead_latency<G>(
    metrics: &G,
    request: &SuccessRequest<'_>,
    overhead_ms: Option<Decimal>,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    if let Some(milliseconds) = overhead_ms.filter(|amount| *amount != Decimal::ZERO) {
        let seconds = milliseconds
            .checked_div(MILLIS_PER_SECOND)
            .ok_or(PrometheusDeploymentError::Overflow)?;
        let labels = prometheus_label_factory(
            metrics.labels_for(DeploymentMetric::OverheadLatency),
            request.values,
            metrics.label_source(request.context),
        )?;
        metrics.observe_histogram(DeploymentMetric::OverheadLatency, &labels, seconds);
    }
    Ok(())
}

fn record_remaining<G>(
    metrics: &G,
    request: &SuccessRequest<'_>,
    payload: &StandardLoggingPayload,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    let Some(headers) = payload
        .hidden_params
        .additional_headers
        .as_ref()
        .filter(|headers| !headers.is_empty())
    else {
        return Ok(());
    };
    let source = metrics.label_source(request.context);
    for (header, metric) in [
        (
            REMAINING_REQUESTS_HEADER,
            DeploymentMetric::RemainingRequests,
        ),
        (REMAINING_TOKENS_HEADER, DeploymentMetric::RemainingTokens),
    ] {
        let Some(amount) = header_amount(headers.get(header), header)? else {
            continue;
        };
        metrics.set_gauge(metric, request.values, source, amount)?;
    }
    Ok(())
}

// a header value may be a number or a numeric string, and prometheus_client sets only a nonzero one
fn header_amount(value: Option<&Json>, field: &str) -> Result<Option<Decimal>, HeaderValueError> {
    let parsed = match value {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Number(number)) => decimal_text(&number.to_string()),
        Some(Json::Text(text)) => decimal_text(text),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            return Err(HeaderValueError::NotANumber {
                field: field.to_owned(),
            });
        }
    };
    match parsed {
        Some(amount) if amount != Decimal::ZERO => Ok(Some(amount)),
        Some(_) => Ok(None),
        None => Err(HeaderValueError::NotANumber {
            field: field.to_owned(),
        }),
    }
}

fn record_latency<G>(
    metrics: &G,
    request: &SuccessRequest<'_>,
    source: LabelSource<'_>,
) -> DeploymentResult<G>
where
    G: DeploymentMetrics,
{
    let response = request
        .end_time
        .checked_sub(request.start_time)
        .ok_or(PrometheusDeploymentError::Overflow)?;
    let first_token = match request.stream {
        Some(true) => Some(
            request
                .completion_start_time
                .unwrap_or(request.end_time)
                .checked_sub(request.start_time)
                .ok_or(PrometheusDeploymentError::Overflow)?,
        ),
        Some(false) | None => None,
    };
    let latency = first_token
        .filter(|seconds| *seconds != Decimal::ZERO)
        .unwrap_or(response);
    let output_tokens = request.output_tokens.unwrap_or(Decimal::ONE);
    if output_tokens <= Decimal::ZERO {
        return Ok(());
    }
    let per_token = latency
        .checked_div(output_tokens)
        .ok_or(PrometheusDeploymentError::Overflow)?;
    let labels = prometheus_label_factory(
        metrics.labels_for(DeploymentMetric::LatencyPerOutputToken),
        request.values,
        source,
    )?;
    metrics.observe_histogram(DeploymentMetric::LatencyPerOutputToken, &labels, per_token);
    Ok(())
}
