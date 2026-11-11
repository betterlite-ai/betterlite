use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;

use super::agent::NewRelicAgent;
use super::backoff::MetricBackoff;
use super::init::{NewRelicInit, NewRelicSettings};
use super::{metrics, success};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::failure_counter::{CallbackFailureCounter, handle_callback_failure};
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::newrelic_extraction::content::ContentPolicy;
use crate::newrelic_extraction::ids::UuidSource;
use crate::newrelic_metrics::clock::Clock;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct NewRelicDeps {
    pub agent: Option<Arc<dyn NewRelicAgent>>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub clock: Clock,
    pub ids: Arc<dyn UuidSource>,
    pub backoff: Arc<MetricBackoff>,
    pub version: Option<String>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
}

pub struct NewRelicLogger {
    pub settings: NewRelicSettings,
    pub enabled: bool,
    pub agent: Option<Arc<dyn NewRelicAgent>>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub clock: Clock,
    pub ids: Arc<dyn UuidSource>,
    pub backoff: Arc<MetricBackoff>,
    pub version: Option<String>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
}

impl NewRelicLogger {
    #[must_use]
    pub fn new(init: &NewRelicInit, deps: NewRelicDeps) -> Arc<Self> {
        let settings = NewRelicSettings::resolve(init, deps.environment.as_ref());
        let enabled = initialize(&settings, deps.agent.as_ref(), deps.environment.as_ref());
        Arc::new(Self {
            settings,
            enabled,
            agent: deps.agent,
            environment: deps.environment,
            clock: deps.clock,
            ids: deps.ids,
            backoff: deps.backoff,
            version: deps.version,
            failure_counters: deps.failure_counters,
        })
    }

    pub(super) fn count_callback_failure(&self) {
        let mut counters: Vec<&dyn CallbackFailureCounter> =
            Vec::with_capacity(self.failure_counters.len());
        for counter in &self.failure_counters {
            counters.push(counter.as_ref());
        }
        handle_callback_failure(self.callback_name(), &counters);
    }

    fn handle_success_event(&self, event: &LoggerEvent) {
        if let Err(error) = success::process_event(self, event) {
            tracing::warn!(%error, "Error in New Relic success event");
            self.count_callback_failure();
        }
    }
}

fn initialize(
    settings: &NewRelicSettings,
    installed_agent: Option<&Arc<dyn NewRelicAgent>>,
    environment: &dyn EnvironmentLookup,
) -> bool {
    let (Some(_license_key), Some(app_name)) =
        (settings.license_key.as_ref(), settings.app_name.as_deref())
    else {
        tracing::warn!(
            "New Relic integration requires NEW_RELIC_LICENSE_KEY and NEW_RELIC_APP_NAME environment variables. Integration will be disabled."
        );
        return false;
    };
    let Some(agent) = installed_agent else {
        tracing::error!(
            "New Relic Python agent not installed. Review the New Relic integration documentation at https://docs.litellm.ai/docs/observability/newrelic."
        );
        return false;
    };
    // timeout=0 is what makes the Python agent connect in its background thread instead of blocking startup
    match agent.register_application(Duration::ZERO) {
        Ok(()) => {
            let policy = ContentPolicy {
                turn_off_message_logging: settings.turn_off_message_logging,
                redact_message_logging: false,
                environment,
            };
            tracing::info!(
                app = app_name,
                content_recording = policy.record_content(),
                "New Relic AI Monitoring initialized"
            );
            true
        }
        Err(error) => {
            tracing::error!(
                %error,
                "Failed to initialize New Relic agent. Integration will be disabled."
            );
            false
        }
    }
}

#[async_trait]
impl CallbackLogger for NewRelicLogger {
    fn callback_name(&self) -> &'static str {
        "newrelic"
    }

    fn turn_off_message_logging(&self) -> bool {
        self.settings.turn_off_message_logging
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.handle_success_event(event);
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.handle_success_event(event.as_ref());
        Ok(())
    }

    fn log_failure_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        metrics::record_error_metric(self);
        Ok(())
    }

    async fn async_log_failure_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        metrics::record_error_metric(self);
        Ok(())
    }
}

impl ApiCallLogger for NewRelicLogger {
    fn log_pre_api_call(&self, _event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        Ok(())
    }

    fn log_post_api_call(&self, _event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}
