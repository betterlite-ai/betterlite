use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus_exporter::PrometheusCollectors;
use betterlite_logging::prometheus_rate_limits::RemainingModelGroupUsage;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::budget::BudgetLookupPort;
use super::error::PrometheusHookError;
use super::metrics::HookMetrics;
use super::provider::{CatalogAdapter, ProviderResolution};
use super::settings::PrometheusHookSettings;

// the name litellm.callbacks lists, so B8655 resolves "prometheus" to this logger
pub(crate) const CALLBACK_NAME: &str = "PrometheusLogger";

// the router readers B3879's remaining gauges and B3880's requested-model bound take; absent when the proxy has no router
#[derive(Clone, Default)]
pub struct PrometheusHookPorts {
    pub router_usage: Option<Arc<dyn RemainingModelGroupUsage>>,
    pub requested_models: Option<Arc<dyn RequestedModelCatalog + Send + Sync>>,
}

pub struct PrometheusHooks {
    pub(crate) collectors: Arc<PrometheusCollectors>,
    pub(crate) settings: PrometheusHookSettings,
    pub(crate) ports: PrometheusHookPorts,
    pub(crate) budget: RemainingBudgetFanOut<Arc<BudgetLookupPort>>,
    pub(crate) budget_lookups: Arc<BudgetLookupPort>,
    pub(crate) provider: ProviderResolution,
}

impl PrometheusHooks {
    #[must_use]
    pub fn new(
        collectors: Arc<PrometheusCollectors>,
        settings: PrometheusHookSettings,
        ports: PrometheusHookPorts,
        budget: RemainingBudgetFanOut<Arc<BudgetLookupPort>>,
        budget_lookups: Arc<BudgetLookupPort>,
    ) -> Self {
        Self {
            collectors,
            settings,
            ports,
            budget,
            budget_lookups,
            provider: ProviderResolution::bundled(),
        }
    }

    pub(crate) fn metrics(&self) -> HookMetrics<'_> {
        HookMetrics::new(
            &self.collectors,
            &self.settings.labels,
            self.settings.end_user,
            &self.settings.limits,
        )
    }

    pub(crate) fn catalog(&self) -> Option<CatalogAdapter> {
        self.ports
            .requested_models
            .as_ref()
            .map(|port| CatalogAdapter(Arc::clone(port)))
    }
}

#[async_trait]
impl CallbackLogger for PrometheusHooks {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_success(Some(&event.payload), &event)
            .await
            .map_err(delivery_error)
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_failure(&event).await;
        Ok(())
    }
}

// the worker logs an error a logger returns, so the hook's own error text travels as the delivery message
fn delivery_error(error: PrometheusHookError) -> LoggerError {
    LoggerError::Delivery {
        callback: CALLBACK_NAME.to_owned(),
        message: error.to_string(),
    }
}
