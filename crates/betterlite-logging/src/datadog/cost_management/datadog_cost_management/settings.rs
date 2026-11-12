use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::{CostManagementError, HeaderError};
use crate::batch::logger::{
    BATCH_SIZE_VARIABLE, BatchSettings, FLUSH_INTERVAL_VARIABLE, SettingsError,
};
use crate::datadog::tagging::Environment;

pub const API_KEY_VARIABLE: &str = "DD_API_KEY";
pub const APP_KEY_VARIABLE: &str = "DD_APP_KEY";
pub const SITE_VARIABLE: &str = "DD_SITE";
pub const DEFAULT_SITE: &str = "datadoghq.com";
pub const COST_TAG_KEYS_SETTING: &str = "cost_tag_keys";
pub const BATCH_SIZE_SETTING: &str = "batch_size";
pub const FLUSH_INTERVAL_SETTING: &str = "flush_interval";
pub const MAX_QUEUE_SIZE_SETTING: &str = "max_queue_size";
pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const API_KEY_HEADER: &str = "dd-api-key";
pub const APP_KEY_HEADER: &str = "dd-application-key";

// CustomLogger takes these for every logger, and the cost logger never reads their effect
const GENERIC_SETTINGS: [&str; 2] = ["turn_off_message_logging", "message_logging"];
const COUNT: &str = "a non-negative whole number";
const SECONDS: &str = "a number of seconds above zero";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CostManagementInit {
    pub cost_tag_keys: Vec<String>,
    pub batch_size: Option<usize>,
    pub flush_interval: Option<Duration>,
    pub max_queue_size: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct CostManagementSettings {
    pub api_key: Option<SecretString>,
    pub app_key: Option<SecretString>,
    pub site: String,
    pub upload_url: String,
    pub cost_tag_keys: Vec<String>,
    pub batch: BatchSettings,
    pub environment: Environment,
}

impl CostManagementInit {
    pub fn from_params(params: &IndexMap<String, String>) -> Result<Self, CostManagementError> {
        let mut init = Self::default();
        for (name, value) in params {
            match name.as_str() {
                COST_TAG_KEYS_SETTING => init.cost_tag_keys = split_keys(value),
                BATCH_SIZE_SETTING => init.batch_size = batch_size(value)?,
                FLUSH_INTERVAL_SETTING => init.flush_interval = flush_interval(value)?,
                MAX_QUEUE_SIZE_SETTING => init.max_queue_size = Some(count(name, value)?),
                other if GENERIC_SETTINGS.contains(&other) => {}
                other => {
                    return Err(CostManagementError::UnknownSetting {
                        name: other.to_owned(),
                    });
                }
            }
        }
        Ok(init)
    }
}

fn split_keys(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

fn count(name: &str, value: &str) -> Result<usize, CostManagementError> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_invalid| CostManagementError::InvalidSetting {
            name: name.to_owned(),
            expected: COUNT,
        })
}

fn batch_size(value: &str) -> Result<Option<usize>, CostManagementError> {
    match count(BATCH_SIZE_SETTING, value)? {
        0 => {
            tracing::warn!(
                setting = BATCH_SIZE_SETTING,
                "Datadog Cost Management: a batch size of 0 uses the default"
            );
            Ok(None)
        }
        size => Ok(Some(size)),
    }
}

fn flush_interval(value: &str) -> Result<Option<Duration>, CostManagementError> {
    let seconds =
        value
            .trim()
            .parse::<f64>()
            .map_err(|_invalid| CostManagementError::InvalidSetting {
                name: FLUSH_INTERVAL_SETTING.to_owned(),
                expected: SECONDS,
            })?;
    if seconds == 0.0_f64 {
        tracing::warn!(
            setting = FLUSH_INTERVAL_SETTING,
            "Datadog Cost Management: a flush interval of 0 uses the default"
        );
        return Ok(None);
    }
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|_invalid| CostManagementError::InvalidSetting {
            name: FLUSH_INTERVAL_SETTING.to_owned(),
            expected: SECONDS,
        })
}

impl CostManagementSettings {
    pub fn resolve(
        init: &CostManagementInit,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, CostManagementError> {
        let tags = Environment::from_lookup(environment)?;
        let credentials = Credentials {
            api_key: secret(environment, API_KEY_VARIABLE),
            app_key: secret(environment, APP_KEY_VARIABLE),
        };
        if credentials.api_key.is_none() || credentials.app_key.is_none() {
            tracing::warn!(
                "Datadog Cost Management: DD_API_KEY and DD_APP_KEY are required. Integration will not work."
            );
        }
        let site = environment
            .lookup(SITE_VARIABLE)
            .filter(|site| !site.is_empty())
            .unwrap_or_else(|| DEFAULT_SITE.to_owned());
        let batch = resolve_batch(init, environment)?;
        Ok(Self {
            upload_url: format!("https://api.{site}/api/v2/cost/custom_costs"),
            api_key: credentials.api_key,
            app_key: credentials.app_key,
            site,
            cost_tag_keys: init.cost_tag_keys.clone(),
            batch,
            environment: tags,
        })
    }

    // LiteLLM builds the headers only after it has both keys, so a missing one skips the upload
    pub fn headers(&self) -> Result<Option<HeaderMap>, HeaderError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
        for (header, secret) in [
            (API_KEY_HEADER, self.api_key.as_ref()),
            (APP_KEY_HEADER, self.app_key.as_ref()),
        ] {
            let Some(text) = secret else {
                return Ok(None);
            };
            let mut header_value = HeaderValue::from_str(text.expose_secret())
                .map_err(|source| HeaderError::Invalid { header, source })?;
            header_value.set_sensitive(true);
            headers.insert(HeaderName::from_static(header), header_value);
        }
        Ok(Some(headers))
    }
}

struct Credentials {
    api_key: Option<SecretString>,
    app_key: Option<SecretString>,
}

fn secret(environment: &dyn EnvironmentLookup, name: &str) -> Option<SecretString> {
    environment
        .lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

fn resolve_batch(
    init: &CostManagementInit,
    environment: &dyn EnvironmentLookup,
) -> Result<BatchSettings, SettingsError> {
    let mut values = BTreeMap::new();
    for name in [BATCH_SIZE_VARIABLE, FLUSH_INTERVAL_VARIABLE] {
        if let Some(value) = environment.lookup(name) {
            values.insert(name.to_owned(), value);
        }
    }
    let mut batch = BatchSettings::from_environment(&values)?;
    // LiteLLM's `batch_size or default` and `flush_interval or default` treat zero as unset
    if let Some(size) = init.batch_size.filter(|size| *size > 0) {
        batch.batch_size = size;
    }
    if let Some(interval) = init.flush_interval.filter(|interval| !interval.is_zero()) {
        batch.flush_interval = interval;
    }
    if let Some(max) = init.max_queue_size {
        batch.max_queue_size = max;
    }
    Ok(batch)
}
