use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DynamicVersionError {
    #[error("the client custom query has no api-version")]
    MissingApiVersion,
}

// LiteLLM compares only a non-None api_version, and reads the key even when the query lacks it
pub fn check_dynamic_azure_params(
    api_version: Option<&str>,
    custom_query: Option<&BTreeMap<String, String>>,
) -> Result<bool, DynamicVersionError> {
    let Some(query) = custom_query else {
        return Ok(true);
    };
    let Some(version) = api_version else {
        return Ok(false);
    };
    let current = query
        .get("api-version")
        .ok_or(DynamicVersionError::MissingApiVersion)?;
    Ok(version != current)
}
