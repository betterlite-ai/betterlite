use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use thiserror::Error;

use crate::datadog_logging::error::DatadogError;
use crate::datadog_logging::logger::{DatadogDeps, DatadogLogger};
use crate::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use crate::dynamic_logger_cache::{DynamicLoggerCache, DynamicLoggerCacheError};

const SERVICE_NAME: &str = "datadog";

pub struct DatadogCredentials {
    pub dd_api_key: Option<String>,
    pub dd_site: Option<String>,
    pub dd_agent_host: Option<String>,
    pub dd_agent_port: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct DatadogTeamSettings {
    pub params: DatadogInitParams,
    pub use_v1: bool,
}

#[derive(Debug, Error)]
pub enum DatadogTeamHandlerError {
    #[error("the team-scoped Datadog logger cache failed")]
    Cache(#[from] DynamicLoggerCacheError),
    #[error("the team-scoped Datadog logger could not be built")]
    Build(#[from] DatadogError),
}

#[must_use]
pub fn datadog_credentials(params: &BTreeMap<String, String>) -> DatadogCredentials {
    DatadogCredentials {
        dd_api_key: params.get("dd_api_key").cloned(),
        dd_site: params.get("dd_site").cloned(),
        dd_agent_host: params.get("dd_agent_host").cloned(),
        dd_agent_port: params.get("dd_agent_port").cloned(),
    }
}

#[must_use]
pub fn datadog_credentials_passed(params: &BTreeMap<String, String>) -> bool {
    params.contains_key("dd_api_key")
        || params.contains_key("dd_site")
        || params.contains_key("dd_agent_host")
}

fn cache_credentials(credentials: &DatadogCredentials) -> BTreeMap<String, SecretString> {
    let mut keyed = BTreeMap::new();
    if let Some(api_key) = &credentials.dd_api_key {
        keyed.insert("dd_api_key".to_owned(), SecretString::from(api_key.clone()));
    }
    if let Some(site) = &credentials.dd_site {
        keyed.insert("dd_site".to_owned(), SecretString::from(site.clone()));
    }
    if let Some(host) = &credentials.dd_agent_host {
        keyed.insert("dd_agent_host".to_owned(), SecretString::from(host.clone()));
    }
    if let Some(port) = &credentials.dd_agent_port {
        keyed.insert("dd_agent_port".to_owned(), SecretString::from(port.clone()));
    }
    keyed
}

pub fn get_datadog_logger_for_request(
    params: &BTreeMap<String, String>,
    cache: &DynamicLoggerCache<Arc<DatadogLogger>>,
    environment: &dyn EnvironmentLookup,
    deps: DatadogDeps,
    settings: DatadogTeamSettings,
) -> Result<Arc<DatadogLogger>, DatadogTeamHandlerError> {
    let credentials = datadog_credentials(params);
    let keyed = cache_credentials(&credentials);
    if let Some(logger) = cache.get_cache(&keyed, SERVICE_NAME)? {
        return Ok(logger);
    }
    let allow_env_credentials =
        credentials.dd_agent_host.is_none() && credentials.dd_site.is_none();
    let init = DatadogInit {
        dd_api_key: credentials.dd_api_key.map(SecretString::from),
        dd_site: credentials.dd_site,
        dd_agent_host: credentials.dd_agent_host,
        dd_agent_port: credentials.dd_agent_port,
        allow_env_credentials,
        params: settings.params,
        use_v1: settings.use_v1,
    };
    let logger = DatadogLogger::new(&init, environment, deps)?;
    cache.set_cache(&keyed, SERVICE_NAME, Arc::clone(&logger))?;
    tracing::debug!("Datadog: Created and cached new DataDogLogger for team-scoped credentials");
    Ok(logger)
}

mod tests;
