use std::time::Instant;

use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::end_user::{
    CostTrackingService,
    get_end_user_id_for_cost_tracking as end_user_for_cost_tracking, // ast-grep-ignore: no-long-names
};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus::failure::invalid_key::should_skip_metrics_for_invalid_key as skip_invalid_key; // ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_deployment::failure::set_llm_deployment_failure_metrics as deployment_failure_metrics; // ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_deployment::requests::FailureRequest;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_usage::counters::UsageLabels;
use betterlite_logging::prometheus_usage::request_spend::increment_zero_cost_requests;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::error::PrometheusHookError;
use super::event::HookCall;
use super::hooks::PrometheusHooks;
use super::labels::{EventLabels, event_values, payload_exception, zero_cost_diagnostic};
use super::metrics::{CounterWrite, HookMetrics};
use super::provider::requested_catalog;

pub(crate) async fn record_failure(
    hooks: &PrometheusHooks,
    payload: Option<&StandardLoggingPayload>,
    event: &LoggerEvent,
) -> Result<(), PrometheusHookError> {
    let call = HookCall::from_event(event);
    let exception = payload.and_then(payload_exception);
    if skip_invalid_key(None, None, exception.as_ref()) {
        return Ok(());
    }
    let end_user = end_user_for_cost_tracking(
        &call.litellm_params,
        CostTrackingService::Prometheus,
        &hooks.settings.end_user,
    )?;
    let labels = EventLabels {
        settings: &hooks.settings.labels,
        payload,
        model: &call.model,
        end_user,
        success: false,
    };
    let values = event_values(&labels)?;
    let metrics = hooks.metrics();
    let now = Instant::now();
    emit_failed_counter(hooks, &values, &metrics, now)?;
    emit_failure_zero_cost(hooks, payload, &values, &metrics)?;
    emit_deployment_failure(hooks, &call, payload, exception.as_ref(), now);
    let org_id = payload.and_then(|record| record.metadata.user_api_key_org_id.as_deref());
    hooks
        .budget_lookups
        .org_after_failure(org_id)
        .await
        .map_err(|error| PrometheusHookError::Budget {
            message: error.message,
        })
}

fn emit_failed_counter(
    hooks: &PrometheusHooks,
    values: &UserApiKeyLabelValues,
    metrics: &HookMetrics<'_>,
    now: Instant,
) -> Result<(), PrometheusHookError> {
    metrics.emit(&CounterWrite {
        counter: &hooks.collectors.litellm_llm_api_failed_requests_metric, // ast-grep-ignore: no-long-names
        metric_name: "litellm_llm_api_failed_requests_metric",
        values,
        source: LabelSource::Values {
            settings: &hooks.settings.labels,
            end_user_settings: hooks.settings.end_user,
        },
        amount: MetricValue::Finite(Decimal::ONE),
        now,
    })
}

fn emit_failure_zero_cost(
    hooks: &PrometheusHooks,
    payload: Option<&StandardLoggingPayload>,
    values: &UserApiKeyLabelValues,
    metrics: &HookMetrics<'_>,
) -> Result<(), PrometheusHookError> {
    let context = PrometheusLabelFactoryContext::new(
        values,
        &hooks.settings.labels,
        hooks.settings.end_user,
    )?;
    let diagnostic = payload.and_then(zero_cost_diagnostic);
    increment_zero_cost_requests(
        metrics,
        diagnostic.as_ref(),
        UsageLabels {
            values,
            context: &context,
            settings: &hooks.settings.labels,
        },
    )?;
    Ok(())
}

fn emit_deployment_failure(
    hooks: &PrometheusHooks,
    call: &HookCall,
    payload: Option<&StandardLoggingPayload>,
    exception: Option<&LabelException>,
    now: Instant,
) {
    let catalog = hooks.catalog();
    deployment_failure_metrics(
        &hooks.metrics(),
        &FailureRequest {
            model: Some(&call.model),
            exception,
            litellm_params: &call.litellm_params,
            payload,
            catalog: requested_catalog(catalog.as_ref()),
            now,
        },
    );
}

mod tests;
