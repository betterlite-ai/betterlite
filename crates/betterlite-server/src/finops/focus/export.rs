use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::focus_export_schedule;
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::focus::engine::source::UsageSource;
use betterlite_logging::finops::focus::engine::{ExportError, ExportSettings, FocusExportEngine};
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use chrono::{DateTime, Utc};
use tokio_util::task::TaskTracker;

use super::error::FocusError;
use super::export_schedule::FocusPodLock;

#[path = "export/tests/constructor_defaults.rs"]
mod constructor_defaults;
#[path = "export/tests/constructor_overrides.rs"]
mod constructor_overrides;
#[path = "export/tests/cron_offset_error.rs"]
mod cron_offset_error;
#[path = "export/tests/engine_adapter.rs"]
mod engine_adapter;
#[path = "export/tests/export_usage_data_bounds.rs"]
mod export_usage_data_bounds;
#[path = "export/tests/export_usage_data_routing.rs"]
mod export_usage_data_routing;
#[path = "export/tests/interval_parsing_invalid.rs"]
mod interval_parsing_invalid;
#[path = "export/tests/prefix_precedence.rs"]
mod prefix_precedence;

// The export engine as the logger sees it: the two entry points the source's FocusExportEngine offers.
pub trait FocusExporter: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn export_all(
        &self,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn export_window(
        &self,
        window: &FocusTimeWindow,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// The source builds the engine on first use from the four values __init__ stored; the composition root supplies this.
pub trait FocusEngineFactory: Send + Sync {
    type Engine: FocusExporter;

    fn create(
        &self,
        settings: ExportSettings,
    ) -> Result<Self::Engine, <Self::Engine as FocusExporter>::Error>;
}

impl<Destination, Source, Clock> FocusExporter for FocusExportEngine<Destination, Source, Clock>
where
    Destination: FocusDestination + Send + Sync,
    Destination::Error: Error + Send + Sync + 'static,
    Source: UsageSource,
    Source::Error: Error + Send + Sync + 'static,
    Clock: UtcClock,
{
    type Error = ExportError<Destination::Error, Source::Error>;

    async fn export_all(&self, limit: Option<i64>) -> Result<(), Self::Error> {
        Self::export_all(self, limit).await
    }

    async fn export_window(
        &self,
        window: &FocusTimeWindow,
        limit: Option<i64>,
    ) -> Result<(), Self::Error> {
        Self::export_window(self, window, limit).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct FocusLoggerParams {
    pub provider: Option<String>,
    pub export_format: Option<String>,
    pub frequency: Option<String>,
    pub cron_offset_minute: Option<i64>,
    pub interval_seconds: Option<i64>,
    pub prefix: Option<String>,
    pub destination_config: Option<DestinationOverrides>,
}

pub struct FocusLoggerDeps<F: FocusEngineFactory> {
    pub factory: F,
    pub clock: Arc<dyn UtcClock>,
    pub pod_lock: Option<Arc<dyn FocusPodLock>>,
    // the server owns this tracker and drains it at shutdown, so a release a cancelled export leaves behind still runs
    pub tasks: TaskTracker,
}

pub struct FocusLogger<F: FocusEngineFactory> {
    pub(crate) provider: String,
    pub(crate) export_format: String,
    pub(crate) frequency: String,
    pub(crate) cron_offset_minute: i64,
    pub(crate) interval_seconds: i64,
    pub(crate) prefix: String,
    pub(crate) destination_config: DestinationOverrides,
    pub(super) pod_lock: Option<Arc<dyn FocusPodLock>>,
    pub(super) clock: Arc<dyn UtcClock>,
    pub(super) tasks: TaskTracker,
    factory: F,
    exporter: Mutex<Option<Arc<F::Engine>>>,
}

impl<F: FocusEngineFactory> FocusLogger<F> {
    pub fn new<L>(
        environment: &L,
        params: FocusLoggerParams,
        deps: FocusLoggerDeps<F>,
    ) -> Result<Self, FocusError<<F::Engine as FocusExporter>::Error>>
    where
        L: EnvironmentLookup + ?Sized,
    {
        let settings = focus_export_schedule::Settings::from_environment(environment)?;
        Ok(Self {
            provider: params
                .provider
                .unwrap_or(settings.inner.provider)
                .to_lowercase(),
            export_format: params
                .export_format
                .unwrap_or(settings.inner.export_format)
                .to_lowercase(),
            frequency: params
                .frequency
                .unwrap_or(settings.inner.frequency)
                .to_lowercase(),
            cron_offset_minute: params
                .cron_offset_minute
                .unwrap_or(settings.inner.cron_offset_minute),
            interval_seconds: params
                .interval_seconds
                .or(settings.inner.interval_seconds)
                .unwrap_or(focus_export_schedule::DEFAULT_INTERVAL_SECONDS),
            prefix: params.prefix.unwrap_or(settings.inner.prefix),
            destination_config: params.destination_config.unwrap_or_default(),
            pod_lock: deps.pod_lock,
            clock: deps.clock,
            tasks: deps.tasks,
            factory: deps.factory,
            exporter: Mutex::new(None),
        })
    }

    // the composition root calls this once scheduling stops: lock releases a cancelled export left behind finish here
    pub async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }

    // _ensure_engine: built on first use, kept afterwards, and built again after a failure
    fn ensure_engine(
        &self,
    ) -> Result<Arc<F::Engine>, FocusError<<F::Engine as FocusExporter>::Error>> {
        let mut cached = self.exporter.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(engine) = cached.as_ref() {
            return Ok(Arc::clone(engine));
        }
        let settings = ExportSettings {
            provider: self.provider.clone(),
            export_format: self.export_format.clone(),
            prefix: self.prefix.clone(),
            destination_config: self.destination_config.clone(),
        };
        let engine = Arc::new(self.factory.create(settings).map_err(FocusError::Export)?);
        *cached = Some(Arc::clone(&engine));
        drop(cached);
        Ok(engine)
    }

    // _export_all/_export_window: the engine is built on first use and the error keeps its cause
    pub async fn export_usage_data(
        &self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> Result<(), FocusError<<F::Engine as FocusExporter>::Error>> {
        match (start_time_utc, end_time_utc) {
            (Some(start_time), Some(end_time)) => {
                let window = FocusTimeWindow {
                    start_time,
                    end_time,
                    frequency: self.frequency.clone(),
                };
                self.ensure_engine()?
                    .export_window(&window, limit)
                    .await
                    .map_err(FocusError::Export)
            }
            (None, None) => self
                .ensure_engine()?
                .export_all(limit)
                .await
                .map_err(FocusError::Export),
            _ => Err(FocusError::UnpairedBounds),
        }
    }
}
