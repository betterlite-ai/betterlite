use std::future::{Future, ready};
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus_exporter::PrometheusCollectors;
use betterlite_logging::prometheus_rate_limits::RemainingModelGroupUsage;
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::budget::BudgetLookupPort;
use super::error::PrometheusHookError;
use super::failure;
use super::metrics::HookMetrics;
use super::provider::{CatalogAdapter, ProviderResolution};
use super::proxy_hooks;
use super::settings::PrometheusHookSettings;
use super::shared;
use super::success;

// the name litellm.callbacks lists, so B8655 resolves "prometheus" to this logger
pub(crate) const CALLBACK_NAME: &str = "PrometheusLogger";

// both event hooks log the kwargs keys litellm passes; the typed event keeps those three parts
const HOOK_KEYS: [&str; 3] = ["standard_logging_object", "litellm_params", "model"];

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

    pub(crate) async fn log_success(
        &self,
        payload: Option<&StandardLoggingPayload>,
        event: &LoggerEvent,
    ) -> Result<(), PrometheusHookError> {
        tracing::debug!(
            "prometheus Logging - Enters success logging function (kwargs keys: {:?})",
            HOOK_KEYS
        );
        let recorded = payload.ok_or(PrometheusHookError::MissingPayload)?;
        success::record_success(self, recorded, event).await
    }

    pub(crate) async fn log_failure(&self, event: &LoggerEvent) {
        tracing::debug!(
            "prometheus Logging - Enters failure logging function (kwargs keys: {:?})",
            HOOK_KEYS
        );
        if let Err(error) = failure::record_failure(self, Some(&event.payload), event).await {
            tracing::error!("prometheus Layer Error(): Exception occurred - {error}");
        }
    }

    pub fn async_post_call_success_hook() -> impl Future<Output = ()> + Send {
        // the total requests counter is counted in the success event, so this hook stays empty
        ready(())
    }

    pub fn async_post_call_failure_hook<'hook>(
        &'hook self,
        request_data: &'hook JsonMap,
        exception: &'hook LabelException,
        auth: &'hook UserAPIKeyAuth,
    ) -> impl Future<Output = ()> + Send + 'hook {
        if let Err(error) = proxy_hooks::record_proxy_failure(self, request_data, exception, auth) {
            tracing::error!("prometheus Layer Error(): Exception occurred - {error}");
        }
        ready(())
    }

    // the first call runs the build, and every later call returns the instance it made
    pub fn shared<Build, BuildError>(build: Build) -> Result<Arc<Self>, BuildError>
    where
        Build: FnOnce() -> Result<Self, BuildError>,
    {
        let mut installed = shared::SHARED
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = installed.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let hooks = Arc::new(build()?);
        *installed = Some(Arc::clone(&hooks));
        drop(installed);
        Ok(hooks)
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
            .map_err(|error| delivery_error(&error))
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_failure(&event).await;
        Ok(())
    }
}

// the worker logs an error a logger returns, so the hook's own error text travels as the delivery message
fn delivery_error(error: &PrometheusHookError) -> LoggerError {
    LoggerError::Delivery {
        callback: CALLBACK_NAME.to_owned(),
        message: error.to_string(),
    }
}
