use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::proxy_provider_options::Raw;

pub const ENV_USE_LITELLM_PROXY: &str = "USE_LITELLM_PROXY";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub environment_use_litellm_proxy: bool,
    pub global_use_litellm_proxy: bool,
    pub deployment_use_litellm_proxy: Vec<bool>,
}

pub fn load_spend_proxy_selection<L>(source: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    project_settings(raw, lookup)
}

#[derive(Debug, Deserialize)]
pub struct PolicyInput {
    #[serde(flatten)]
    selection: Raw,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendAttributionError {
    #[error("invalid spend proxy selection: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

pub fn decode_spend_proxy_selection(bytes: &[u8]) -> Result<Settings, SpendAttributionError> {
    let input: PolicyInput = serde_json::from_slice(bytes)?;
    Ok(project_settings(input.selection, &input.environment)?)
}

fn project_settings<L: EnvironmentLookup>(raw: Raw, lookup: &L) -> Result<Settings, ConfigError> {
    let loaded = raw.convert(lookup)?;
    Ok(Settings {
        environment_use_litellm_proxy: loaded.environment_use_litellm_proxy,
        global_use_litellm_proxy: loaded.global_use_litellm_proxy,
        deployment_use_litellm_proxy: loaded.deployment_use_litellm_proxy,
    })
}

mod tests;
