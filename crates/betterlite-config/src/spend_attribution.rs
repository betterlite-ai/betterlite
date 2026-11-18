mod tests;

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const ENV_USE_LITELLM_PROXY: &str = "USE_LITELLM_PROXY";
const GLOBAL_USE_LITELLM_PROXY_PATH: &str = "litellm_settings.use_litellm_proxy";
const DEPLOYMENT_PROXY_FLAG_PATH: &str = "model_list[].litellm_params.use_litellm_proxy";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    litellm_settings: GlobalSettings,
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GlobalSettings {
    use_litellm_proxy: JsonValue,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Deployment {
    litellm_params: DeploymentParams,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeploymentParams {
    use_litellm_proxy: JsonValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub environment_use_litellm_proxy: bool,
    pub global_use_litellm_proxy: bool,
    // One flat entry per model_list row, in config order; rows that name no flag carry false.
    pub deployment_use_litellm_proxy: Vec<bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = lookup.lookup(ENV_USE_LITELLM_PROXY);
        let mut deployment_use_litellm_proxy = Vec::with_capacity(self.model_list.len());
        for deployment in self.model_list {
            deployment_use_litellm_proxy.push(configured_flag(
                DEPLOYMENT_PROXY_FLAG_PATH,
                &deployment.litellm_params.use_litellm_proxy,
            )?);
        }
        Ok(Settings {
            environment_use_litellm_proxy: environment_flag(environment.as_deref())?,
            global_use_litellm_proxy: configured_flag(
                GLOBAL_USE_LITELLM_PROXY_PATH,
                &self.litellm_settings.use_litellm_proxy,
            )?,
            deployment_use_litellm_proxy,
        })
    }
}

pub fn load_spend_proxy_selection<L>(source: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    Section::convert(raw, lookup)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PolicyInput {
    pub litellm_settings: GlobalSettings,
    pub model_list: Vec<Deployment>,
    pub environment: BTreeMap<String, String>,
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
    let raw = Raw {
        litellm_settings: input.litellm_settings,
        model_list: input.model_list,
    };
    Ok(Section::convert(raw, &input.environment)?)
}

fn environment_flag(value: Option<&str>) -> Result<bool, ConfigError> {
    value.map_or(Ok(false), |text| {
        betterlite_env::flag(ENV_USE_LITELLM_PROXY, text)
            .map(|flag| flag.unwrap_or(false))
            .map_err(|source| ConfigError::Environment { source })
    })
}

fn configured_flag(path: &str, value: &JsonValue) -> Result<bool, ConfigError> {
    match value {
        JsonValue::Null => Ok(false),
        JsonValue::Bool(flag) => Ok(*flag),
        JsonValue::String(text) => betterlite_env::flag(path, text)
            .map(|flag| flag.unwrap_or(false))
            .map_err(|_| ConfigError::InvalidScalar {
                path: path.to_owned(),
                value: false,
            }),
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: path.to_owned(),
                value: false,
            })
        }
    }
}
