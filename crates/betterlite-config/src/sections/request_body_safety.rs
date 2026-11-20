use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::provider_url_destination::pattern::AllowedDestination;

mod tests;

pub const MAX_FALLBACKS_VARIABLE: &str = "ROUTER_MAX_FALLBACKS";
pub const DEFAULT_MAX_FALLBACKS: i64 = 5;
const ALLOWED_HOSTS_PATH: &str = "litellm_settings.provider_url_destination_allowed_hosts";
const GRANTS_PATH: &str = "configurable_clientside_auth_params";

// general_settings.allow_client_side_credentials is compared with `is True`, so only a YAML true opts in
#[derive(Debug, Default, Deserialize)]
struct GeneralSettings {
    #[serde(default)]
    allow_client_side_credentials: Option<bool>,
}

// litellm_settings is set raw on the litellm module, and the host matcher accepts a bare string as one entry
#[derive(Debug, Default, Deserialize)]
struct LiteLlmSettings {
    #[serde(default, rename = "provider_url_destination_allowed_hosts")]
    destination_hosts: Option<Hosts>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Hosts {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    litellm_params: LiteLlmParams,
}

#[derive(Debug, Default, Deserialize)]
struct LiteLlmParams {
    #[serde(default, rename = "configurable_clientside_auth_params")]
    clientside_params: Option<Vec<Json>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: GeneralSettings,
    #[serde(default)]
    litellm_settings: LiteLlmSettings,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

// CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS: a string grants its parameter, a mapping only api_base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientsideParamGrant {
    Exact(String),
    Matcher(JsonObject),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub allow_client_side_credentials: bool,
    pub allowed_hosts: Vec<AllowedDestination>,
    pub model_grants: BTreeMap<String, Option<Vec<ClientsideParamGrant>>>,
    pub max_fallbacks: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            allow_client_side_credentials: false,
            allowed_hosts: Vec::new(),
            model_grants: BTreeMap::new(),
            max_fallbacks: DEFAULT_MAX_FALLBACKS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // int(ROUTER_MAX_FALLBACKS) at import: a non-number stops LiteLLM from starting, and no value is clamped
        let text = lookup.lookup(MAX_FALLBACKS_VARIABLE).unwrap_or_default();
        let max_fallbacks = betterlite_env::number::<i64>(MAX_FALLBACKS_VARIABLE, &text)?
            .unwrap_or(DEFAULT_MAX_FALLBACKS);
        Ok(Settings {
            allow_client_side_credentials: self
                .general_settings
                .allow_client_side_credentials
                .unwrap_or(false),
            allowed_hosts: allowed_hosts(self.litellm_settings.destination_hosts, lookup)?,
            model_grants: model_grants(self.model_list)?,
            max_fallbacks,
        })
    }
}

fn allowed_hosts<L>(raw: Option<Hosts>, lookup: &L) -> Result<Vec<AllowedDestination>, ConfigError>
where
    L: EnvironmentLookup,
{
    let entries = match raw {
        None => Vec::new(),
        Some(Hosts::One(host)) => vec![host],
        Some(Hosts::Many(hosts)) => hosts,
    };
    let resolved = entries
        .into_iter()
        .map(|entry| resolve_string(entry, ALLOWED_HOSTS_PATH, lookup))
        .collect::<Result<Vec<String>, ConfigError>>()?;
    Ok(resolved
        .iter()
        .filter_map(|host| AllowedDestination::parse(host))
        .collect())
}

// get_model_group_info keeps the last matching deployment's value, including None, so the last entry wins here
fn model_grants(
    deployments: Vec<Deployment>,
) -> Result<BTreeMap<String, Option<Vec<ClientsideParamGrant>>>, ConfigError> {
    model_grant_map(deployments.into_iter().map(|deployment| {
        (
            deployment.model_name,
            deployment.litellm_params.clientside_params,
        )
    }))
}

// The same mapping over another caller's declarations, so the oracle helper shares this loader's rules.
pub fn model_grant_map<D>(
    deployments: D,
) -> Result<BTreeMap<String, Option<Vec<ClientsideParamGrant>>>, ConfigError>
where
    D: IntoIterator<Item = (Option<String>, Option<Vec<Json>>)>,
{
    let mut grants = BTreeMap::new();
    for (index, (model_name, raw)) in deployments.into_iter().enumerate() {
        let Some(name) = model_name else {
            continue;
        };
        let converted = raw
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| grant(item, index))
                    .collect::<Result<Vec<ClientsideParamGrant>, ConfigError>>()
            })
            .transpose()?;
        grants.insert(name, converted);
    }
    Ok(grants)
}

fn grant(item: Json, deployment: usize) -> Result<ClientsideParamGrant, ConfigError> {
    match item {
        Json::Text(name) => Ok(ClientsideParamGrant::Exact(name)),
        Json::Object(fields) => Ok(ClientsideParamGrant::Matcher(fields)),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(ConfigError::InvalidScalar {
                path: format!("model_list[{deployment}].litellm_params.{GRANTS_PATH}"),
                value: matches!(other, Json::Bool(true)),
            })
        }
    }
}
