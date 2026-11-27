use std::time::Instant;

use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::end_user::{
    CostTrackingService,
    get_end_user_id_for_cost_tracking as end_user_for_cost_tracking, // ast-grep-ignore: no-long-names
};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus::failure::invalid_key::should_skip_metrics_for_invalid_key as skip_invalid_key; // ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_deployment::requests::SuccessRequest;
use betterlite_logging::prometheus_deployment::success::set_llm_deployment_success_metrics as deployment_success_metrics; // ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_latency::input_sequence_length::input_sequence_length_bucket;
use betterlite_logging::prometheus_latency::labels::LabelInputs;
use betterlite_logging::prometheus_latency::latency_histograms::{
    LatencyObservation, LatencyTimestamps, set_latency_metrics,
};
use betterlite_logging::prometheus_rate_limits::key_and_team::set_key_and_team_rate_limit_metrics as key_and_team_rate_limits; // ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_rate_limits::router_remaining::{
    RouterRemainingInputs,
    async_set_router_remaining_metrics as router_remaining_metrics, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus_rate_limits::virtual_key::{
    VirtualKeyRateLimitInputs,
    set_virtual_key_rate_limit_metrics as virtual_key_rate_limits, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus_usage::cache::increment_cache_metrics;
use betterlite_logging::prometheus_usage::counters::UsageLabels;
use betterlite_logging::prometheus_usage::mcp::increment_mcp_tool_calls;
use betterlite_logging::prometheus_usage::media::increment_media_metrics;
use betterlite_logging::prometheus_usage::request_spend::{
    increment_request_and_spend, increment_zero_cost_requests,
};
use betterlite_logging::prometheus_usage::tokens::increment_token_metrics;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::error::PrometheusHookError;
use super::event::{HookCall, fan_out_metadata};
use super::hooks::PrometheusHooks;
use super::json::{input_length_inputs, queue_time_seconds};
use super::labels::{
    EventLabels, event_values, hashed_key, payload_exception, zero_cost_diagnostic,
};
use super::latency::now_seconds;
use super::metrics::{CounterWrite, HookMetrics};
use crate::prometheus_remaining_budget::fan_out::RemainingBudgetRequest;

const SUCCESS_KEYS: [&str; 3] = ["standard_logging_object", "litellm_params", "model"];

// the parts every emitter step reads, built once after the label values and a shared label context
struct SuccessRecord<'record> {
    metrics: HookMetrics<'record>,
    call: &'record HookCall,
    payload: &'record StandardLoggingPayload,
    exception: Option<&'record LabelException>,
    values: &'record UserApiKeyLabelValues,
    context: &'record PrometheusLabelFactoryContext,
    labels: UsageLabels<'record>,
    now: Instant,
}

impl PrometheusHooks {
    pub(crate) async fn log_success(
        &self,
        payload: Option<&StandardLoggingPayload>,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        tracing::debug!(
            "prometheus Logging - Enters success logging function (kwargs keys: {:?})",
            SUCCESS_KEYS
        );
        let recorded = payload.ok_or(PrometheusHookError::MissingPayload)?;
        self.record_success(recorded, event).await
    }

    async fn record_success(
        &self,
        payload: &StandardLoggingPayload,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        let call = HookCall::from_event(event);
        let exception = payload_exception(payload);
        if skip_invalid_key(None, None, exception.as_ref()) {
            return Ok(());
        }
        let end_user = end_user_for_cost_tracking(
            &call.litellm_params,
            CostTrackingService::Prometheus,
            &self.settings.end_user,
        )?;
        let labels = EventLabels {
            settings: &self.settings.labels,
            payload: Some(payload),
            model: &call.model,
            end_user,
            success: true,
        };
        let values = event_values(&labels)?;
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &self.settings.labels,
            self.settings.end_user,
        )?;
        let record = SuccessRecord {
            metrics: self.metrics(),
            call: &call,
            payload,
            exception: exception.as_ref(),
            values: &values,
            context: &context,
            labels: UsageLabels {
                values: &values,
                context: &context,
                settings: &self.settings.labels,
            },
            now: Instant::now(),
        };
        let emitter_key = hashed_key(values.hashed_api_key.as_deref());
        usage_phase(&record)?;
        self.record_budget(&record, emitter_key.as_deref()).await;
        self.record_rate_limits(&record, emitter_key.as_deref())?;
        self.record_latency(&record, queue_time_seconds(&call.metadata))?;
        deployment_phase(&record);
        self.record_router(&record).await;
        usage_tail_phase(&record)
    }

    // the budget fan-out runs on the hashed key, and the caller's own labels keep the payload's value
    async fn record_budget(&self, record: &SuccessRecord<'_>, emitter_key: Option<&str>) {
        let values = record.values;
        let fan_out = fan_out_metadata(&record.call.metadata);
        let request = RemainingBudgetRequest {
            team: values.team.as_deref(),
            team_alias: values.team_alias.as_deref(),
            key: emitter_key,
            key_alias: values.api_key_alias.as_deref(),
            user_id: values.user.as_deref(),
            org_id: values.org_id.as_deref(),
            end_user_id: values.end_user.as_deref(),
            response_cost: record.payload.response_cost,
            metadata: &fan_out,
        };
        self.budget
            .increment_remaining_budget_metrics(request)
            .await;
    }

    fn record_rate_limits(
        &self,
        record: &SuccessRecord<'_>,
        emitter_key: Option<&str>,
    ) -> Result<(), PrometheusHookError> {
        virtual_key_rate_limits(
            &record.metrics,
            VirtualKeyRateLimitInputs {
                user_api_key: emitter_key,
                user_api_key_alias: record.values.api_key_alias.as_deref(),
                model_group: record.values.model_group.as_deref(),
                model_id: record.values.model_id.as_deref(),
                metadata: &record.call.metadata,
                payload: Some(record.payload),
                settings: &self.settings.labels,
                end_user_settings: self.settings.end_user,
            },
        )?;
        key_and_team_rate_limits(
            &record.metrics,
            record.payload,
            LabelInputs {
                values: record.values,
                settings: &self.settings.labels,
                end_user_settings: self.settings.end_user,
                context: Some(record.context),
            },
        )?;
        Ok(())
    }

    fn record_latency(
        &self,
        record: &SuccessRecord<'_>,
        queue_time: Option<Decimal>,
    ) -> Result<(), PrometheusHookError> {
        let label_enabled = self.collectors.emit_input_sequence_length_label; // ast-grep-ignore: no-long-names
        let input_sequence_length = label_enabled
            .then(|| input_sequence_length_bucket(&input_length_inputs(record.payload)));
        let timestamps = LatencyTimestamps {
            start_time: Some(record.payload.start_time),
            end_time: Some(record.payload.end_time),
            // the event carries no api-call start, so the request start stands in for it
            api_call_start: Some(record.payload.start_time),
            completion_start: (record.payload.completion_start_time != Decimal::ZERO)
                .then_some(record.payload.completion_start_time),
        };
        set_latency_metrics(
            &self.latency_histograms(),
            &LatencyObservation {
                timestamps,
                now: now_seconds(),
                now_monotonic: record.now,
                stream: record.payload.stream,
                queue_time_seconds: queue_time,
                input_sequence_length,
            },
            &LabelInputs {
                values: record.values,
                settings: &self.settings.labels,
                end_user_settings: self.settings.end_user,
                context: Some(record.context),
            },
        )?;
        Ok(())
    }

    async fn record_router(&self, record: &SuccessRecord<'_>) {
        router_remaining_metrics(
            &record.metrics,
            RouterRemainingInputs {
                payload: record.payload,
                values: record.values,
                context: Some(record.context),
                settings: &self.settings.labels,
                end_user_settings: self.settings.end_user,
                usage: self.ports.router_usage.as_deref(),
            },
        )
        .await;
    }
}

fn usage_phase(record: &SuccessRecord<'_>) -> Result<(), PrometheusHookError> {
    increment_request_and_spend(&record.metrics, record.payload, record.labels, record.now)?;
    zero_cost_phase(record)?;
    increment_token_metrics(&record.metrics, record.payload, record.labels, record.now)
}

fn zero_cost_phase(record: &SuccessRecord<'_>) -> Result<(), PrometheusHookError> {
    // litellm writes the zero-cost counter between the request and spend counters and the token counters
    let diagnostic = zero_cost_diagnostic(record.payload);
    increment_zero_cost_requests(&record.metrics, diagnostic.as_ref(), record.labels)
}

fn usage_tail_phase(record: &SuccessRecord<'_>) -> Result<(), PrometheusHookError> {
    increment_cache_metrics(&record.metrics, record.payload, record.labels, record.now)?;
    increment_media_metrics(&record.metrics, record.payload, record.labels, record.now)?;
    increment_mcp_tool_calls(
        &record.metrics,
        record.payload,
        record.labels,
        record.payload.response_cost,
        record.now,
    )?;
    record.metrics.emit(CounterWrite {
        counter: &record
            .metrics
            .collectors
            .litellm_proxy_total_requests_metric, // ast-grep-ignore: no-long-names
        metric_name: "litellm_proxy_total_requests_metric",
        values: record.values,
        source: LabelSource::Context(record.context),
        amount: MetricValue::Finite(Decimal::ONE),
        now: record.now,
    })
}

fn deployment_phase(record: &SuccessRecord<'_>) {
    deployment_success_metrics(
        &record.metrics,
        &SuccessRequest {
            model: Some(&record.call.model),
            stream: record.payload.stream,
            completion_start_time: (record.payload.completion_start_time != Decimal::ZERO)
                .then_some(record.payload.completion_start_time),
            litellm_params: &record.call.litellm_params,
            payload: Some(record.payload),
            exception: record.exception,
            values: record.values,
            context: Some(record.context),
            start_time: record.payload.start_time,
            end_time: record.payload.end_time,
            output_tokens: Some(Decimal::from(record.payload.completion_tokens)),
            now: record.now,
        },
    );
}

mod tests;
