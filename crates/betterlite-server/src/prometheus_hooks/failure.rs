use std::time::Instant;

use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::end_user::{CostTrackingService, get_end_user_id_for_cost_tracking};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::invalid_key::should_skip_metrics_for_invalid_key;
use betterlite_logging::prometheus_deployment::failure::set_llm_deployment_failure_metrics;
use betterlite_logging::prometheus_deployment::requests::FailureRequest;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_usage::counters::UsageLabels;
use betterlite_logging::prometheus_usage::request_spend::increment_zero_cost_requests;

use super::error::PrometheusHookError;
use super::event::HookCall;
use super::hooks::PrometheusHooks;
use super::labels::{failure_values, payload_exception, zero_cost_diagnostic};

const FAILURE_KEYS: [&str; 3] = ["standard_logging_object", "litellm_params", "model"];

impl PrometheusHooks {
    // ast-grep-ignore: no-long-names
    pub(crate) async fn log_failure(&self, event: &LoggerEvent) {
        tracing::debug!(
            "prometheus Logging - Enters failure logging function (kwargs keys: {:?})",
            FAILURE_KEYS
        );
        if let Err(error) = self.record_failure(Some(&event.payload), event).await {
            tracing::error!("prometheus Layer Error(): Exception occured - {error}");
        }
    }

    async fn record_failure(
        &self,
        payload: Option<&StandardLoggingPayload>,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        let call = HookCall::from_event(event);
        let exception = payload.and_then(payload_exception);
        if should_skip_metrics_for_invalid_key(None, None, exception.as_ref()) {
            return Ok(());
        }
        let end_user = get_end_user_id_for_cost_tracking(
            &call.litellm_params,
            CostTrackingService::Prometheus,
            &self.settings.end_user,
        )?;
        let values = failure_values(&self.settings.labels, payload, &call.model, end_user)?;
        let metrics = self.metrics();
        let now = Instant::now();
        metrics.emit_one(
            &self.collectors.litellm_llm_api_failed_requests_metric,
            "litellm_llm_api_failed_requests_metric",
            &values,
            LabelSource::Values {
                settings: &self.settings.labels,
                end_user_settings: self.settings.end_user,
            },
            now,
        )?;
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &self.settings.labels,
            self.settings.end_user,
        )?;
        let diagnostic = payload.and_then(zero_cost_diagnostic);
        increment_zero_cost_requests(
            &metrics,
            diagnostic.as_ref(),
            UsageLabels {
                values: &values,
                context: &context,
                settings: &self.settings.labels,
            },
        )?;
        let catalog = self.catalog();
        set_llm_deployment_failure_metrics(
            &metrics,
            &FailureRequest {
                model: Some(&call.model),
                exception: exception.as_ref(),
                litellm_params: &call.litellm_params,
                payload,
                catalog: catalog
                    .as_ref()
                    .map(|adapter| adapter as &dyn RequestedModelCatalog),
                now,
            },
        );
        self.budget_lookups
            .org_after_failure(values.org_id.as_deref())
            .await
            .map_err(|error| PrometheusHookError::Budget {
                message: error.message,
            })?;
        Ok(())
    }
}

mod tests;
