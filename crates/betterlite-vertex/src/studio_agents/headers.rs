use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::StudioAgentError;
use super::request::AgentParams;
use crate::studio_files::resolve_key;

pub type AgentHeaders = IndexMap<String, SecretString>;

pub fn validate_environment(
    caller: &BTreeMap<String, String>,
    params: &AgentParams,
    settings: &Settings,
) -> Result<AgentHeaders, StudioAgentError> {
    let custom_base = params
        .api_base
        .as_deref()
        .is_some_and(|base| !base.is_empty());
    if custom_base
        && params
            .api_key
            .as_ref()
            .is_none_or(|key| key.expose_secret().is_empty())
    {
        return Err(StudioAgentError::CustomBaseWithoutKey);
    }
    let key =
        resolve_key(&[params.api_key.as_ref()], settings).ok_or(StudioAgentError::MissingApiKey)?;
    let mut headers: AgentHeaders = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    headers.insert("x-goog-api-key".to_owned(), key);
    Ok(headers)
}
