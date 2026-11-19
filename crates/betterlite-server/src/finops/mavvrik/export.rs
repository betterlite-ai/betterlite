use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::focus_export_schedule;
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::focus::engine::ExportSettings;
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use betterlite_logging::focus_records::transformer::UsageRow;
use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use super::error::ConfigError;
use crate::finops::focus::export_schedule::FocusPodLock;

#[path = "export/tests/frequency_daily_only.rs"]
mod frequency_daily_only;
#[path = "export/tests/init_super_call.rs"]
mod init_super_call;
#[path = "export/tests/is_empty_marker_cases.rs"]
mod is_empty_marker_cases;
#[path = "export/tests/max_rows_parsing.rs"]
mod max_rows_parsing;
#[path = "export/tests/parse_marker_formats.rs"]
mod parse_marker_formats;
#[path = "export/tests/parse_marker_none.rs"]
mod parse_marker_none;
#[path = "export/tests/parse_marker_warning.rs"]
mod parse_marker_warning;

pub(super) const MAVVRIK_FOCUS_EXPORT_JOB_NAME: &str = "mavvrik_focus_export_usage_data";
pub const DEFAULT_MAX_ROWS: i64 = 500_000;

const FREQUENCY_VARIABLE: &str = "MAVVRIK_FOCUS_FREQUENCY";
const MAX_ROWS_VARIABLE: &str = "MAVVRIK_FOCUS_MAX_ROWS";
const API_KEY_VARIABLE: &str = "MAVVRIK_API_KEY";
const API_ENDPOINT_VARIABLE: &str = "MAVVRIK_API_ENDPOINT";
const CONNECTION_ID_VARIABLE: &str = "MAVVRIK_CONNECTION_ID";

// The Mavvrik destination and engine as the logger drives them for one window: register, query, encode, name, deliver.
pub trait MavvrikExporter: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn metrics_marker(&self) -> impl Future<Output = Result<MetricsMarker, Self::Error>> + Send;

    fn usage_rows(
        &self,
        limit: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, Self::Error>> + Send;

    // empty when the transform yields no rows, so the caller can deliver it as an empty export
    fn payload(&self, rows: &[UsageRow]) -> Result<Vec<u8>, Self::Error>;

    fn filename(&self, window: &FocusTimeWindow) -> Result<String, Self::Error>;

    fn deliver(
        &self,
        content: &[u8],
        window: &FocusTimeWindow,
        filename: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

pub trait MavvrikEngineFactory: Send + Sync {
    type Engine: MavvrikExporter;

    fn create(
        &self,
        settings: ExportSettings,
    ) -> Result<Self::Engine, <Self::Engine as MavvrikExporter>::Error>;
}

pub struct MavvrikLoggerDeps {
    pub clock: Arc<dyn UtcClock>,
    pub pod_lock: Option<Arc<dyn FocusPodLock>>,
    pub tasks: TaskTracker,
}

pub struct MavvrikFocusLogger<F: MavvrikEngineFactory> {
    provider: String,
    export_format: String,
    pub(super) frequency: String,
    prefix: String,
    destination_config: DestinationOverrides,
    pub(super) cron_offset_minute: i64,
    pub(super) max_rows: i64,
    pub(super) pod_lock: Option<Arc<dyn FocusPodLock>>,
    pub(super) clock: Arc<dyn UtcClock>,
    pub(super) tasks: TaskTracker,
    factory: F,
    exporter: Mutex<Option<Arc<F::Engine>>>,
}

impl<F: MavvrikEngineFactory> MavvrikFocusLogger<F> {
    // MavvrikFocusLogger.__init__: the daily check runs before the FocusLogger settings it passes to super
    pub fn new(
        environment: &dyn EnvironmentLookup,
        factory: F,
        deps: MavvrikLoggerDeps,
    ) -> Result<Self, ConfigError> {
        let frequency = daily_frequency(environment)?;
        let base = focus_export_schedule::Settings::from_environment(environment)?;
        Ok(Self {
            provider: String::from("mavvrik"),
            export_format: String::from("csv"),
            frequency,
            prefix: String::from("mavvrik_focus_exports"),
            destination_config: destination_settings(environment),
            cron_offset_minute: base.inner.cron_offset_minute,
            max_rows: max_rows_setting(environment)?,
            pod_lock: deps.pod_lock,
            clock: deps.clock,
            tasks: deps.tasks,
            factory,
            exporter: Mutex::new(None),
        })
    }

    // _ensure_engine: built on first use and kept afterwards
    pub(super) fn ensure_engine(
        &self,
    ) -> Result<Arc<F::Engine>, <F::Engine as MavvrikExporter>::Error> {
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
        let engine = Arc::new(self.factory.create(settings)?);
        *cached = Some(Arc::clone(&engine));
        drop(cached);
        Ok(engine)
    }

    // the composition root calls this once scheduling stops: lock releases a cancelled export left behind finish here
    pub async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

// os.getenv("MAVVRIK_FOCUS_FREQUENCY", "daily").lower(), with an empty value counting as unset
fn daily_frequency(environment: &dyn EnvironmentLookup) -> Result<String, ConfigError> {
    let frequency = environment
        .lookup(FREQUENCY_VARIABLE)
        .filter(|value| !value.is_empty())
        .map_or_else(|| String::from("daily"), |value| value.to_lowercase());
    if frequency == "daily" {
        return Ok(frequency);
    }
    Err(ConfigError::UnsupportedFrequency { frequency })
}

// int(MAVVRIK_FOCUS_MAX_ROWS) if set, else 500_000; an invalid value fails construction
fn max_rows_setting(environment: &dyn EnvironmentLookup) -> Result<i64, ConfigError> {
    let Some(raw) = environment.lookup(MAX_ROWS_VARIABLE) else {
        return Ok(DEFAULT_MAX_ROWS);
    };
    Ok(betterlite_env::number::<i64>(MAX_ROWS_VARIABLE, &raw)?.unwrap_or(DEFAULT_MAX_ROWS))
}

// the destination_config dict super().__init__ receives: missing values stay None for the destination to reject
fn destination_settings(environment: &dyn EnvironmentLookup) -> DestinationOverrides {
    DestinationOverrides {
        bucket_name: None,
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        api_key: environment
            .lookup(API_KEY_VARIABLE)
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        integration_token: None,
        base_url: None,
        service_account_json: None,
        api_endpoint: environment
            .lookup(API_ENDPOINT_VARIABLE)
            .filter(|value| !value.is_empty()),
        connection_id: environment
            .lookup(CONNECTION_ID_VARIABLE)
            .filter(|value| !value.is_empty()),
    }
}

// _parse_metrics_marker: an empty marker is no date at all, a Unix timestamp becomes UTC midnight of its date, and a full ISO datetime keeps its time
pub(super) fn parse_metrics_marker(marker: &MetricsMarker) -> Option<DateTime<Utc>> {
    if is_empty_metrics_marker(marker) {
        return None;
    }
    match marker {
        MetricsMarker::Epoch(seconds) => parse_epoch(*seconds),
        MetricsMarker::Text(text) => {
            let trimmed = text.trim();
            if let Some(parsed) = parse_text(trimmed) {
                return Some(parsed);
            }
            tracing::warn!(
                "Mavvrik FOCUS: could not parse metricsMarker {trimmed:?} — skipping catch-up"
            );
            None
        }
    }
}

fn parse_epoch(seconds: i64) -> Option<DateTime<Utc>> {
    let Some(parsed) = DateTime::from_timestamp(seconds, 0) else {
        tracing::warn!(
            "Mavvrik FOCUS: could not parse metricsMarker {seconds} — skipping catch-up"
        );
        return None;
    };
    Some(parsed.date_naive().and_time(NaiveTime::MIN).and_utc())
}

fn parse_text(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(date.and_time(NaiveTime::MIN).and_utc());
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%SZ") {
        return Some(parsed.and_utc());
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|parsed| parsed.and_utc())
}

// _is_empty_metrics_marker: 0 and blank text mean no data has been ingested yet
pub(super) fn is_empty_metrics_marker(marker: &MetricsMarker) -> bool {
    match marker {
        MetricsMarker::Epoch(seconds) => *seconds == 0,
        MetricsMarker::Text(text) => text.trim().is_empty(),
    }
}
