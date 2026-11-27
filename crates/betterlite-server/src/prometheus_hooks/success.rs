use std::time::Instant;

use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::end_user::{CostTrackingService, get_end_user_id_for_cost_tracking};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus::failure::invalid_key::should_skip_metrics_for_invalid_key;
use betterlite_logging::prometheus_deployment::requests::SuccessRequest;
use betterlite_logging::prometheus_deployment::success::set_llm_deployment_success_metrics;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_latency::input_sequence_length::input_sequence_length_bucket;
use betterlite_logging::prometheus_latency::labels::LabelInputs;
use betterlite_logging::prometheus_latency::latency_histograms::{
    LatencyObservation, LatencyTimestamps, set_latency_metrics,
};
use betterlite_logging::prometheus_rate_limits::key_and_team::set_key_and_team_rate_limit_metrics;
use betterlite_logging::prometheus_rate_limits::router_remaining::{
    RouterRemainingInputs, async_set_router_remaining_metrics,
};
use betterlite_logging::prometheus_rate_limits::virtual_key::{
    VirtualKeyRateLimitInputs, set_virtual_key_rate_limit_metrics,
};
use betterlite_logging::prometheus_usage::cache::increment_cache_metrics;
use betterlite_logging::prometheus_usage::counters::UsageLabels;
use betterlite_logging::prometheus_usage::mcp::increment_mcp_tool_calls;
use betterlite_logging::prometheus_usage::media::increment_media_metrics;
use betterlite_logging::prometheus_usage::request_spend::{
    increment_request_and_spend, increment_zero_cost_requests,
};
use betterlite_logging::prometheus_usage::tokens::increment_token_metrics;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::error::PrometheusHookError;
use super::event::{HookCall, fan_out_metadata};
use super::hooks::PrometheusHooks;
use super::json::{input_length_inputs, queue_time_seconds};
use super::labels::{hashed_key, payload_exception, success_values, zero_cost_diagnostic};
use super::latency::{completion_start_time, now_seconds};
use crate::prometheus_remaining_budget::fan_out::RemainingBudgetRequest;

const SUCCESS_KEYS: [&str; 3] = ["standard_logging_object", "litellm_params", "model"];

// the parts every emitter step reads, built once after the label values and a shared label context
struct SuccessRecord<'record> {
    call: &'record HookCall,
    payload: &'record StandardLoggingPayload,
    exception: Option<&'record LabelException>,
    values: &'record UserApiKeyLabelValues,
    context: &'record PrometheusLabelFactoryContext,
}

impl PrometheusHooks {
    // ast-grep-ignore: no-long-names
    pub(crate) async fn log_success(
        &self,
        payload: Option<&StandardLoggingPayload>,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        tracing::debug!(
            "prometheus Logging - Enters success logging function (kwargs keys: {:?})",
            SUCCESS_KEYS
        );
        let payload = payload.ok_or(PrometheusHookError::MissingPayload)?;
        self.record_success(payload, event).await
    }

    async fn record_success(
        &self,
        payload: &StandardLoggingPayload,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        let call = HookCall::from_event(event);
        let exception = payload_exception(payload);
        if should_skip_metrics_for_invalid_key(None, None, exception.as_ref()) {
            return Ok(());
        }
        let end_user = get_end_user_id_for_cost_tracking(
            &call.litellm_params,
            CostTrackingService::Prometheus,
            &self.settings.end_user,
        )?;
        let values = success_values(&self.settings.labels, payload, &call.model, end_user)?;
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &self.settings.labels,
            self.settings.end_user,
        )?;
        let record = SuccessRecord {
            call: &call,
            payload,
            exception: exception.as_ref(),
            values: &values,
            context: &context,
        };
        let emitter_key = hashed_key(values.hashed_api_key.as_deref());
        let now = Instant::now();
        let labels = UsageLabels {
            values: &values,
            context: &context,
            settings: &self.settings.labels,
        };
        self.record_usage(&record, labels, now)?;
        self.record_budget(&record, emitter_key.as_deref()).await;
        self.record_rate_limits(&record, emitter_key.as_deref())?;
        self.record_latency(&record, queue_time_seconds(&call.metadata), now)?;
        self.record_deployment(&record, now);
        self.record_router(&record).await;
        self.record_usage_tail(&record, labels, now)
    }

    fn record_usage(
        &self,
        record: &SuccessRecord<'_>,
        labels: UsageLabels<'_>,
        now: Instant,
    ) -> Result<(), PrometheusHookError> {
        let metrics = self.metrics();
        increment_request_and_spend(&metrics, record.payload, labels, now)?;
        let diagnostic = zero_cost_diagnostic(record.payload);
        increment_zero_cost_requests(&metrics, diagnostic.as_ref(), labels)?;
        increment_token_metrics(&metrics, record.payload, labels, now)?;
        Ok(())
    }

    // the budget fan-out runs on the hashed key, and the caller's own labels keep the payload's value
    async fn record_budget(&self, record: &SuccessRecord<'_>, emitter_key: Option<&str>) {
        let fan_out = fan_out_metadata(&record.call.metadata);
        self.budget
            .increment_remaining_budget_metrics(RemainingBudgetRequest {
                team: record.values.team.as_deref(),
                team_alias: record.values.team_alias.as_deref(),
                key: emitter_key,
                key_alias: record.values.api_key_alias.as_deref(),
                user_id: record.values.user.as_deref(),
                org_id: record.values.org_id.as_deref(),
                end_user_id: record.values.end_user.as_deref(),
                response_cost: record.payload.response_cost,
                metadata: &fan_out,
            })
            .await;
    }

    fn record_rate_limits(
        &self,
        record: &SuccessRecord<'_>,
        emitter_key: Option<&str>,
    ) -> Result<(), PrometheusHookError> {
        let metrics = self.metrics();
        set_virtual_key_rate_limit_metrics(
            &metrics,
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
        set_key_and_team_rate_limit_metrics(
            &metrics,
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
        now: Instant,
    ) -> Result<(), PrometheusHookError> {
        let input_sequence_length = self
            .collectors
            .emit_input_sequence_length_label
            .then(|| input_sequence_length_bucket(&input_length_inputs(record.payload)));
        set_latency_metrics(
            &self.latency_histograms(),
            &LatencyObservation {
                timestamps: LatencyTimestamps {
                    start_time: Some(record.payload.start_time),
                    end_time: Some(record.payload.end_time),
                    // the event carries no api-call start, so the request start stands in for it
                    api_call_start: Some(record.payload.start_time),
                    completion_start: completion_start_time(record.payload),
                },
                now: now_seconds(),
                now_monotonic: now,
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

    fn record_deployment(&self, record: &SuccessRecord<'_>, now: Instant) {
        set_llm_deployment_success_metrics(
            &self.metrics(),
            &SuccessRequest {
                model: Some(&record.call.model),
                stream: record.payload.stream,
                completion_start_time: completion_start_time(record.payload),
                litellm_params: &record.call.litellm_params,
                payload: Some(record.payload),
                exception: record.exception,
                values: record.values,
                context: Some(record.context),
                start_time: record.payload.start_time,
                end_time: record.payload.end_time,
                output_tokens: Some(Decimal::from(record.payload.completion_tokens)),
                now,
            },
        );
    }

    async fn record_router(&self, record: &SuccessRecord<'_>) {
        async_set_router_remaining_metrics(
            &self.metrics(),
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

    fn record_usage_tail(
        &self,
        record: &SuccessRecord<'_>,
        labels: UsageLabels<'_>,
        now: Instant,
    ) -> Result<(), PrometheusHookError> {
        let metrics = self.metrics();
        increment_cache_metrics(&metrics, record.payload, labels, now)?;
        increment_media_metrics(&metrics, record.payload, labels, now)?;
        increment_mcp_tool_calls(
            &metrics,
            record.payload,
            labels,
            record.payload.response_cost,
            now,
        )?;
        metrics.emit_one(
            &self.collectors.litellm_proxy_total_requests_metric,
            "litellm_proxy_total_requests_metric",
            record.values,
            LabelSource::Context(record.context),
            now,
        )
    }
}

mod tests;
