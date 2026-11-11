use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::SecretString;
use thiserror::Error;

use crate::dynamic_logger_cache::{DynamicLoggerCache, DynamicLoggerCacheError};
use crate::newrelic_metrics::NewRelicMetricsError;
use crate::newrelic_metrics::logger::{NewRelicDeps, NewRelicMetricsLogger};

const SERVICE_NAME: &str = "newrelic";

pub struct NewRelicCredentials {
    pub newrelic_api_key: Option<String>,
    pub newrelic_region: Option<String>,
}

#[derive(Debug, Error)]
pub enum NewRelicTeamHandlerError {
    #[error("the team-scoped New Relic logger cache failed")]
    Cache(#[from] DynamicLoggerCacheError),
    #[error("the team-scoped New Relic metrics logger could not be built")]
    Build(#[from] NewRelicMetricsError),
}

#[must_use]
pub fn newrelic_credentials(params: &BTreeMap<String, String>) -> NewRelicCredentials {
    NewRelicCredentials {
        newrelic_api_key: params.get("newrelic_api_key").cloned(),
        newrelic_region: params.get("newrelic_region").cloned(),
    }
}

#[must_use]
pub fn newrelic_credentials_passed(params: &BTreeMap<String, String>) -> bool {
    // presence, not truthiness: the dispatcher's empty-string test is the separate caller check
    params.contains_key("newrelic_api_key")
}

fn cache_credentials(credentials: &NewRelicCredentials) -> BTreeMap<String, SecretString> {
    let mut keyed = BTreeMap::new();
    if let Some(api_key) = &credentials.newrelic_api_key {
        keyed.insert(
            "newrelic_api_key".to_owned(),
            SecretString::from(api_key.clone()),
        );
    }
    if let Some(region) = &credentials.newrelic_region {
        keyed.insert(
            "newrelic_region".to_owned(),
            SecretString::from(region.clone()),
        );
    }
    keyed
}

// LiteLLM's get_newrelic_logger_for_request exceeds the 30-character name limit, so it loses the get_ prefix
pub fn newrelic_logger_for_request(
    params: &BTreeMap<String, String>,
    cache: &DynamicLoggerCache<Arc<NewRelicMetricsLogger>>,
    deps: NewRelicDeps,
) -> Result<Arc<NewRelicMetricsLogger>, NewRelicTeamHandlerError> {
    let credentials = newrelic_credentials(params);
    let keyed = cache_credentials(&credentials);
    if let Some(logger) = cache.get_cache(&keyed, SERVICE_NAME)? {
        return Ok(logger);
    }
    let api_key = credentials.newrelic_api_key.unwrap_or_default();
    let logger = NewRelicMetricsLogger::new(
        SecretString::from(api_key),
        credentials.newrelic_region.as_deref(),
        deps,
    )?;
    cache.set_cache(&keyed, SERVICE_NAME, Arc::clone(&logger))?;
    tracing::debug!(
        "New Relic: Created and cached new NewRelicMetricsLogger for team-scoped credentials"
    );
    Ok(logger)
}

mod tests;
