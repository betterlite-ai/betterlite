use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::registry::identity::{Attribute, CallbackIdentity};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use crate::finops::focus::error::ScheduleError;
use crate::finops::focus::export::{
    FocusEngineFactory, FocusLogger, FocusLoggerDeps, FocusLoggerParams,
};
use crate::finops::focus::export_schedule::{
    FocusPodLock, ScheduleTrigger, build_scheduler_trigger,
};

use super::error::VantageExportError;
use super::export_schedule::initialize_vantage_export_job;
use super::settings::{
    EXPORT_FORMAT, EXPORT_PREFIX, PROVIDER, VantageLoggerParams, VantageSettings,
    masked_integration_token,
};

#[path = "export/tests/constructor_base_url.rs"]
mod constructor_base_url;
#[path = "export/tests/constructor_credentials.rs"]
mod constructor_credentials;
#[path = "export/tests/constructor_debug_log.rs"]
mod constructor_debug_log;
#[path = "export/tests/constructor_env_defaults.rs"]
mod constructor_env_defaults;
#[path = "export/tests/constructor_frequency.rs"]
mod constructor_frequency;
#[path = "export/tests/constructor_interval_parsing.rs"]
mod constructor_interval_parsing;
#[path = "export/tests/constructor_parent_call.rs"]
mod constructor_parent_call;

// VantageLogger: FocusLogger configured for Vantage's CSV upload destination.
pub struct VantageLogger<F: FocusEngineFactory> {
    pub settings: VantageSettings,
    pub focus: FocusLogger<F>,
    pub pod_lock: Option<Arc<dyn FocusPodLock>>,
    pub clock: Arc<dyn UtcClock>,
    pub tasks: TaskTracker,
}

// The live instances every Vantage consumer shares: the startup scheduler, settings deletion and the export routes.
pub trait VantageExportLogger: Send + Sync {
    // LiteLLM's registration key: the class name and the public scalar attributes, which is how a list drops a duplicate
    fn registration_identity(&self) -> CallbackIdentity;

    fn scheduler_trigger(&self) -> Result<ScheduleTrigger, ScheduleError>;

    fn initialize_focus_export_job(&self) -> BoxFuture<'_, Result<(), VantageExportError>>;

    fn export_usage_data<'logger>(
        &'logger self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> BoxFuture<'logger, Result<(), VantageExportError>>;
}

impl<F: FocusEngineFactory> VantageLogger<F> {
    // super().__init__ with the Vantage provider, the CSV format and the resolved credentials
    pub fn new<L>(
        environment: &L,
        params: VantageLoggerParams,
        deps: FocusLoggerDeps<F>,
    ) -> Result<Self, VantageExportError>
    where
        L: EnvironmentLookup + ?Sized,
    {
        let settings = VantageSettings::resolve(&params, environment);
        let pod_lock = deps.pod_lock.as_ref().map(Arc::clone);
        let clock = Arc::clone(&deps.clock);
        let tasks = deps.tasks.clone();
        let focus = FocusLogger::new(
            environment,
            FocusLoggerParams {
                provider: Some(PROVIDER.to_owned()),
                export_format: Some(EXPORT_FORMAT.to_owned()),
                frequency: Some(settings.frequency.clone()),
                cron_offset_minute: params.cron_offset_minute,
                interval_seconds: settings.interval_seconds,
                prefix: Some(EXPORT_PREFIX.to_owned()),
                destination_config: Some(settings.destination_overrides()),
            },
            deps,
        )?;
        tracing::debug!(
            "VantageLogger initialized (integration_token={})",
            masked_integration_token(settings.integration_token.as_ref())
        );
        Ok(Self {
            settings,
            focus,
            pod_lock,
            clock,
            tasks,
        })
    }

    // the composition root calls this once scheduling stops: a release a cancelled export left behind finishes here
    pub async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

impl<F> VantageExportLogger for VantageLogger<F>
where
    F: FocusEngineFactory + 'static,
    F::Engine: 'static,
{
    fn registration_identity(&self) -> CallbackIdentity {
        CallbackIdentity {
            class_name: "VantageLogger".to_owned(),
            attributes: IndexMap::from([
                (
                    "provider".to_owned(),
                    Attribute::Text(self.focus.provider.clone()),
                ),
                (
                    "export_format".to_owned(),
                    Attribute::Text(self.focus.export_format.clone()),
                ),
                (
                    "frequency".to_owned(),
                    Attribute::Text(self.focus.frequency.clone()),
                ),
                (
                    "cron_offset_minute".to_owned(),
                    Attribute::Integer(self.focus.cron_offset_minute),
                ),
                (
                    "interval_seconds".to_owned(),
                    Attribute::Integer(self.focus.interval_seconds),
                ),
                (
                    "prefix".to_owned(),
                    Attribute::Text(self.focus.prefix.clone()),
                ),
            ]),
        }
    }

    fn scheduler_trigger(&self) -> Result<ScheduleTrigger, ScheduleError> {
        build_scheduler_trigger(&self.focus)
    }

    fn initialize_focus_export_job(&self) -> BoxFuture<'_, Result<(), VantageExportError>> {
        Box::pin(initialize_vantage_export_job(self))
    }

    fn export_usage_data<'logger>(
        &'logger self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> BoxFuture<'logger, Result<(), VantageExportError>> {
        Box::pin(async move {
            self.focus
                .export_usage_data(limit, start_time_utc, end_time_utc)
                .await
                .map_err(VantageExportError::from)
        })
    }
}
