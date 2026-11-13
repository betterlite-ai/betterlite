use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::{As, DefaultOnNull};

use crate::batch::logger::{
    BatchSettings, DEFAULT_FLUSH_INTERVAL_SECONDS, DEFAULT_MAX_QUEUE_SIZE, FLUSH_INTERVAL_VARIABLE,
};
use crate::datadog::tagging::Environment;
use crate::exporter_options::options::CustomLoggerOptions;

use super::error::DatadogError;

pub const DD_MAX_BATCH_SIZE: usize = 1000;
pub const DD_MAX_PAYLOAD_SIZE_BYTES: usize = 4_000_000;
pub const DEFAULT_AGENT_PORT: &str = "10518";

const BATCH_SIZE_VARIABLE: &str = "DD_BATCH_SIZE";
const AGENT_HOST_VARIABLE: &str = "LITELLM_DD_AGENT_HOST";
const AGENT_PORT_VARIABLE: &str = "LITELLM_DD_AGENT_PORT";
const API_KEY_VARIABLE: &str = "DD_API_KEY";
const SITE_VARIABLE: &str = "DD_SITE";

// litellm.datadog_params holds DatadogInitParams, whose only field is this flag
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DatadogInitParams {
    #[serde(deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub turn_off_message_logging: bool,
}

#[derive(Clone, Debug)]
pub struct DatadogInit {
    pub dd_api_key: Option<SecretString>,
    pub dd_site: Option<String>,
    pub dd_agent_host: Option<String>,
    pub dd_agent_port: Option<String>,
    pub allow_env_credentials: bool,
    pub params: DatadogInitParams,
    pub use_v1: bool,
}

impl Default for DatadogInit {
    fn default() -> Self {
        Self {
            dd_api_key: None,
            dd_site: None,
            dd_agent_host: None,
            dd_agent_port: None,
            allow_env_credentials: true,
            params: DatadogInitParams::default(),
            use_v1: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DatadogSettings {
    pub intake_url: String,
    pub api_key: Option<SecretString>,
    pub batch: BatchSettings,
    pub options: CustomLoggerOptions,
    pub use_v1: bool,
    pub environment: Environment,
}

impl DatadogSettings {
    pub fn resolve(
        init: &DatadogInit,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, DatadogError> {
        let tags = Environment::from_lookup(environment)?;
        let (endpoint_url, api_key) = endpoint(init, environment)?;
        let flush_seconds = environment
            .lookup(FLUSH_INTERVAL_VARIABLE)
            .map(|text| betterlite_env::number::<u64>(FLUSH_INTERVAL_VARIABLE, &text))
            .transpose()?
            .flatten()
            .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS);
        let intake_url = tags
            .base_url
            .as_deref()
            .filter(|base| !base.is_empty())
            .map_or(endpoint_url, |base| format!("{base}/api/v2/logs"));
        Ok(Self {
            intake_url,
            api_key,
            batch: BatchSettings {
                batch_size: resolve_batch_size(environment),
                flush_interval: Duration::from_secs(flush_seconds),
                max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            },
            options: CustomLoggerOptions {
                turn_off_message_logging: init.params.turn_off_message_logging,
                message_logging: true,
            },
            use_v1: init.use_v1,
            environment: tags,
        })
    }
}

fn endpoint(
    init: &DatadogInit,
    environment: &dyn EnvironmentLookup,
) -> Result<(String, Option<SecretString>), DatadogError> {
    let agent_host = init.dd_agent_host.clone().or_else(|| {
        environment
            .lookup(AGENT_HOST_VARIABLE)
            .filter(|host| !host.is_empty())
    });
    if let Some(host) = agent_host {
        let port = init
            .dd_agent_port
            .clone()
            .or_else(|| {
                environment
                    .lookup(AGENT_PORT_VARIABLE)
                    .filter(|port| !port.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_AGENT_PORT.to_owned());
        let api_key = init
            .dd_api_key
            .clone()
            .or_else(|| environment_key(init, environment));
        return Ok((format!("http://{host}:{port}/api/v2/logs"), api_key));
    }
    let resolved_api_key = init
        .dd_api_key
        .clone()
        .or_else(|| environment_key(init, environment));
    let resolved_site = init.dd_site.clone().or_else(|| {
        environment
            .lookup(SITE_VARIABLE)
            .filter(|site| !site.is_empty())
    });
    let Some(api_key) = resolved_api_key else {
        return Err(DatadogError::MissingApiKey);
    };
    let Some(site) = resolved_site else {
        return Err(DatadogError::MissingSite);
    };
    Ok((
        format!("https://http-intake.logs.{site}/api/v2/logs"),
        Some(api_key),
    ))
}

fn environment_key(
    init: &DatadogInit,
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    if !init.allow_env_credentials {
        return None;
    }
    environment
        .lookup(API_KEY_VARIABLE)
        .filter(|key| !key.is_empty())
        .map(SecretString::from)
}

#[must_use]
pub fn resolve_batch_size(environment: &dyn EnvironmentLookup) -> usize {
    let Some(raw) = environment.lookup(BATCH_SIZE_VARIABLE) else {
        return DD_MAX_BATCH_SIZE;
    };
    // docs/compatibility.md reads an empty environment variable as unset
    if raw.is_empty() {
        return DD_MAX_BATCH_SIZE;
    }
    let Some(value) = parse_count(raw.trim()) else {
        tracing::warn!(
            variable = BATCH_SIZE_VARIABLE,
            value = %raw,
            default = DD_MAX_BATCH_SIZE,
            "Datadog: ignoring invalid DD_BATCH_SIZE={raw}, using the default {DD_MAX_BATCH_SIZE}"
        );
        return DD_MAX_BATCH_SIZE;
    };
    let ceiling = i64::try_from(DD_MAX_BATCH_SIZE).unwrap_or(i64::MAX);
    usize::try_from(value.clamp(1, ceiling)).unwrap_or(1)
}

fn parse_count(text: &str) -> Option<i64> {
    let (negative, digits) = text.strip_prefix('-').map_or_else(
        || (false, text.strip_prefix('+').unwrap_or(text)),
        |rest| (true, rest),
    );
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Python's int() has no bound, so an overflow clamps at the end of its sign's range
    Some(
        text.parse::<i64>()
            .unwrap_or(if negative { i64::MIN } else { i64::MAX }),
    )
}
