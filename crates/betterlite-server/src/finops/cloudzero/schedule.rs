use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::finops::cloudzero::delivery::transport::{BillingTransport, Clock};
use jiff::tz::TimeZoneDatabase;
use secrecy::SecretString;

use self::error::CloudZeroSettingsError;
use self::lease::UsageExportLease;

pub mod background;
pub mod dry_run;
pub mod error;
pub mod export;
pub mod lease;
pub mod usage_rows;

mod tests;

pub const EXPORT_JOB_NAME: &str = "cloudzero_export_usage_data";
pub const DEFAULT_TIMEZONE: &str = "UTC";
pub const DEFAULT_INTERVAL_MINUTES: i64 = 60;
pub const DEFAULT_MAX_FETCHED_RECORDS: i64 = 50_000;

const API_KEY_ENV: &str = "CLOUDZERO_API_KEY";
const CONNECTION_ID_ENV: &str = "CLOUDZERO_CONNECTION_ID";
const TIMEZONE_ENV: &str = "CLOUDZERO_TIMEZONE";
const INTERVAL_ENV: &str = "CLOUDZERO_EXPORT_INTERVAL_MINUTES";
const MAX_RECORDS_ENV: &str = "CLOUDZERO_MAX_FETCHED_DATA_RECORDS";

// CloudZero's billing drop operations, the two values its API accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExportOperation {
    #[default]
    ReplaceHourly,
    Sum,
}

impl ExportOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReplaceHourly => "replace_hourly",
            Self::Sum => "sum",
        }
    }
}

// CloudZeroLogger.__init__: parameters first, then the environment, then the defaults.
#[derive(Debug, Clone)]
pub struct CloudZeroSettings {
    pub api_key: Option<SecretString>,
    pub connection_id: Option<SecretString>,
    pub timezone: String,
    pub interval_minutes: i64,
    pub max_fetched_records: i64,
}

impl CloudZeroSettings {
    pub fn resolve(
        api_key: Option<&str>,
        connection_id: Option<&str>,
        timezone: Option<&str>,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, CloudZeroSettingsError> {
        Ok(Self {
            api_key: configured_text(api_key, API_KEY_ENV, environment).map(SecretString::from),
            connection_id: configured_text(connection_id, CONNECTION_ID_ENV, environment)
                .map(SecretString::from),
            timezone: configured_text(timezone, TIMEZONE_ENV, environment)
                .unwrap_or_else(|| DEFAULT_TIMEZONE.to_owned()),
            interval_minutes: configured_number(
                INTERVAL_ENV,
                DEFAULT_INTERVAL_MINUTES,
                environment,
            )?,
            max_fetched_records: configured_number(
                MAX_RECORDS_ENV,
                DEFAULT_MAX_FETCHED_RECORDS,
                environment,
            )?,
        })
    }
}

// The collaborators an export needs; the transport and the clock are cloned per CloudZeroStreamer.
pub struct ExportPipeline<Sender, Time> {
    pub store: PostgresStore,
    pub timezone_database: TimeZoneDatabase,
    pub clock: Time,
    pub transport: Sender,
}

pub struct CloudZeroLogger<Sender, Time> {
    pub settings: CloudZeroSettings,
    pub pipeline: ExportPipeline<Sender, Time>,
    pub lease: Option<Arc<dyn UsageExportLease>>,
}

impl<Sender, Time> CallbackLogger for CloudZeroLogger<Sender, Time>
where
    Sender: BillingTransport + Send + Sync,
    Time: Clock,
{
    fn callback_name(&self) -> &str {
        "cloudzero"
    }
}

fn configured_text(
    parameter: Option<&str>,
    name: &str,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    parameter
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| environment.lookup(name).filter(|value| !value.is_empty()))
}

fn configured_number(
    name: &str,
    default: i64,
    environment: &dyn EnvironmentLookup,
) -> Result<i64, CloudZeroSettingsError> {
    let Some(text) = environment.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::number::<i64>(name, &text)?.unwrap_or(default))
}
