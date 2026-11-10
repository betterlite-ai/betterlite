use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::langfuse_settings::Settings;
use secrecy::SecretString;

use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::{DEFAULT_EXPORT_PATH, EXPORT_PATH_VARIABLE};
use crate::langfuse_settings::Credentials;
use crate::langfuse_settings::export::{max_retries, timeout_seconds};

// Everything the exporter needs from the logger's settings and resolved credentials, all read once here
#[derive(Debug)]
pub struct ExporterOptions {
    pub public_key: Option<String>,
    pub secret_key: Option<SecretString>,
    pub base_url: String,
    pub export_path: String,
    pub timeout: Duration,
    pub retries: u32,
}

pub fn exporter_options(
    settings: &Settings,
    environment: &dyn EnvironmentLookup,
    credentials: &Credentials,
) -> Result<ExporterOptions, LangfuseExporterError> {
    let seconds = timeout_seconds(settings)?;
    let timeout = Duration::try_from_secs_f64(seconds).map_err(|source| {
        LangfuseExporterError::UnusableTimeout {
            raw: settings.timeout.clone().unwrap_or_default(),
            source,
        }
    })?;
    Ok(ExporterOptions {
        public_key: credentials.public_key.clone(),
        secret_key: credentials.secret_key.clone(),
        base_url: credentials.host.clone(),
        export_path: environment
            .lookup(EXPORT_PATH_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_EXPORT_PATH.to_owned()),
        timeout,
        retries: max_retries(settings),
    })
}
