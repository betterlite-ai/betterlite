use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::batch::logger::{
    BATCH_SIZE_VARIABLE, BatchSettings, DEFAULT_BATCH_SIZE, DEFAULT_FLUSH_INTERVAL_SECONDS,
    DEFAULT_MAX_QUEUE_SIZE,
};

use super::error::MetricsError;

pub const DEFAULT_SITE: &str = "datadoghq.com";
pub const DEFAULT_ENV: &str = "unknown";
pub const DEFAULT_SERVICE: &str = "litellm-server";
pub const DEFAULT_VERSION: &str = "unknown";
pub const DEFAULT_POD_NAME: &str = "unknown";

const API_KEY_VARIABLE: &str = "DD_API_KEY";
const APP_KEY_VARIABLE: &str = "DD_APP_KEY";
const SITE_VARIABLE: &str = "DD_SITE";
const ENV_VARIABLE: &str = "DD_ENV";
const SERVICE_VARIABLE: &str = "DD_SERVICE";
const VERSION_VARIABLE: &str = "DD_VERSION";
const HOSTNAME_VARIABLE: &str = "HOSTNAME";
const POD_NAME_VARIABLE: &str = "POD_NAME";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsEnvironment {
    pub env: String,
    pub service: String,
    pub version: String,
    pub hostname: String,
    pub pod_name: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MetricsInit {
    // LiteLLM's constructor forwards a batch_size kwarg; the proxy builds the logger without one
    pub batch_size: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct MetricsSettings {
    pub api_key: Option<SecretString>,
    pub app_key: Option<SecretString>,
    pub upload_url: String,
    pub batch: BatchSettings,
    pub environment: MetricsEnvironment,
}

impl MetricsSettings {
    pub fn resolve(
        environment: &dyn EnvironmentLookup,
        init: &MetricsInit,
    ) -> Result<Self, MetricsError> {
        let api_key = credential(environment, API_KEY_VARIABLE);
        if api_key.is_none() {
            tracing::warn!(
                "Datadog Metrics: DD_API_KEY is required, so the metric uploads are disabled"
            );
        }
        let application_key = credential(environment, APP_KEY_VARIABLE);
        let site = text(environment, SITE_VARIABLE, DEFAULT_SITE);
        let batch_size = match init.batch_size {
            Some(batch_size) => batch_size,
            None => batch_size_from_environment(environment)?,
        };
        Ok(Self {
            api_key,
            app_key: application_key,
            upload_url: format!("https://api.{site}/api/v2/series"),
            batch: BatchSettings {
                batch_size,
                // DatadogMetricsLogger fixes the interval at five seconds, and only the batch size follows the env
                flush_interval: Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS),
                max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            },
            environment: MetricsEnvironment {
                env: text(environment, ENV_VARIABLE, DEFAULT_ENV),
                service: text(environment, SERVICE_VARIABLE, DEFAULT_SERVICE),
                version: text(environment, VERSION_VARIABLE, DEFAULT_VERSION),
                hostname: text(environment, HOSTNAME_VARIABLE, ""),
                pod_name: text(environment, POD_NAME_VARIABLE, DEFAULT_POD_NAME),
            },
        })
    }
}

// docs/compatibility.md reads an empty environment variable as unset, so the default applies
fn text(environment: &dyn EnvironmentLookup, name: &str, default: &str) -> String {
    match environment.lookup(name) {
        Some(value) if !value.is_empty() => value,
        Some(_) | None => default.to_owned(),
    }
}

fn credential(environment: &dyn EnvironmentLookup, name: &str) -> Option<SecretString> {
    environment
        .lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

fn batch_size_from_environment(environment: &dyn EnvironmentLookup) -> Result<usize, MetricsError> {
    let Some(text) = environment.lookup(BATCH_SIZE_VARIABLE) else {
        return Ok(DEFAULT_BATCH_SIZE);
    };
    Ok(betterlite_env::number::<usize>(BATCH_SIZE_VARIABLE, &text)?.unwrap_or(DEFAULT_BATCH_SIZE))
}
