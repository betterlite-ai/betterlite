use std::collections::BTreeMap;
use std::time::Duration;

use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::batch::logger::{BatchSettings, SettingsError};
use crate::opik::config::get_config_variable;
use crate::opik::config_file::ConfigFileError;

pub const DEFAULT_PROJECT_NAME: &str = "Default Project";
pub const DEFAULT_BASE_URL: &str = "https://www.comet.com/opik/api";
pub const TRACE_PATH: &str = "/v1/private/traces/batch";
pub const SPAN_PATH: &str = "/v1/private/spans/batch";
pub const WORKSPACE_HEADER: &str = "comet-workspace";
pub const AUTHORIZATION_HEADER: &str = "authorization";
pub const CONTENT_TYPE_JSON: &str = "application/json";

// LoggerOptions keeps no Debug derive: its api key is plain text until resolve_settings wraps it in a SecretString.
#[derive(Clone, Copy, Default)]
pub struct LoggerOptions<'values> {
    pub project_name: Option<&'values str>,
    pub url: Option<&'values str>,
    pub api_key: Option<&'values str>,
    pub workspace: Option<&'values str>,
    pub batch_size: Option<usize>,
    pub flush_interval: Option<u64>,
    pub max_queue_size: Option<usize>,
}

pub struct OpikSettings {
    pub project_name: String,
    pub trace_url: Url,
    pub span_url: Url,
    pub headers: HeaderMap,
    pub batch: BatchSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum OpikSettingsError {
    #[error("the Opik configuration file could not be read")]
    ConfigFile(#[from] ConfigFileError),
    #[error("the Opik batch settings are invalid")]
    Batch(#[from] SettingsError),
    #[error("the Opik {name} setting is not a valid URL")]
    Url {
        name: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header could not be built")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
}

struct ResolvedValues {
    project_name: String,
    base_url: String,
    api_key: Option<SecretString>,
    workspace: Option<String>,
}

pub(crate) fn resolve_settings(
    options: &LoggerOptions<'_>,
    environment: &BTreeMap<String, String>,
    config_file: Option<&str>,
) -> Result<OpikSettings, OpikSettingsError> {
    let non_empty_environment = without_empty_values(environment);
    let values = resolve_values(options, &non_empty_environment, config_file)?;
    let trace_url = endpoint(&values.base_url, TRACE_PATH)?;
    let span_url = endpoint(&values.base_url, SPAN_PATH)?;
    let headers = opik_headers(values.workspace.as_deref(), values.api_key.as_ref())?;
    let batch = batch_settings(options, &non_empty_environment)?;
    Ok(OpikSettings {
        project_name: values.project_name,
        trace_url,
        span_url,
        headers,
        batch,
    })
}

fn resolve_values(
    options: &LoggerOptions<'_>,
    environment: &BTreeMap<String, String>,
    config_file: Option<&str>,
) -> Result<ResolvedValues, OpikSettingsError> {
    let project_name = get_config_variable(
        "project_name",
        options.project_name,
        environment,
        config_file,
        Some(DEFAULT_PROJECT_NAME),
    )?
    .unwrap_or_else(|| DEFAULT_PROJECT_NAME.to_owned());
    let base_url = get_config_variable(
        "url_override",
        options.url,
        environment,
        config_file,
        Some(DEFAULT_BASE_URL),
    )?
    .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let api_key = get_config_variable("api_key", options.api_key, environment, config_file, None)?
        .map(SecretString::from);
    let workspace = get_config_variable(
        "workspace",
        options.workspace,
        environment,
        config_file,
        None,
    )?;
    Ok(ResolvedValues {
        project_name,
        base_url,
        api_key,
        workspace,
    })
}

// docs/compatibility.md: an empty env var counts as unset, so the Opik config file and the default answer instead
fn without_empty_values(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    environment
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn endpoint(base: &str, path: &str) -> Result<Url, OpikSettingsError> {
    Url::parse(&format!("{base}{path}")).map_err(|source| OpikSettingsError::Url {
        name: "url_override",
        source,
    })
}

fn opik_headers(
    workspace: Option<&str>,
    api_key: Option<&SecretString>,
) -> Result<HeaderMap, OpikSettingsError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
    if let Some(team) = workspace {
        let value = HeaderValue::try_from(team).map_err(|source| OpikSettingsError::Header {
            name: "Comet-Workspace",
            source,
        })?;
        headers.insert(HeaderName::from_static(WORKSPACE_HEADER), value);
    }
    if let Some(key) = api_key {
        let mut value = HeaderValue::try_from(key.expose_secret()).map_err(|source| {
            OpikSettingsError::Header {
                name: "authorization",
                source,
            }
        })?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static(AUTHORIZATION_HEADER), value);
    }
    Ok(headers)
}

fn batch_settings(
    options: &LoggerOptions<'_>,
    environment: &BTreeMap<String, String>,
) -> Result<BatchSettings, SettingsError> {
    let mut batch = BatchSettings::from_environment(environment)?;
    if let Some(batch_size) = options.batch_size {
        batch.batch_size = batch_size;
    }
    if let Some(seconds) = options.flush_interval {
        batch.flush_interval = Duration::from_secs(seconds);
    }
    if let Some(max_queue_size) = options.max_queue_size {
        batch.max_queue_size = max_queue_size;
    }
    Ok(batch)
}
