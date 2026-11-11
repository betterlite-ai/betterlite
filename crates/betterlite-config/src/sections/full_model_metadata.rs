use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

pub const DEFAULT_MAX_LRU_CACHE_SIZE: i64 = 64;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(rename = "DEFAULT_MAX_LRU_CACHE_SIZE")]
    default_max_lru_cache_size: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(rename = "DEFAULT_MAX_LRU_CACHE_SIZE")]
    default_max_lru_cache_size: Option<RawNumber>,
}

// deployments stay index-aligned with model_list, like the router's config order
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub default_max_lru_cache_size: i64,
    pub deployment_max_lru_cache_sizes: Box<[i64]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_max_lru_cache_size: DEFAULT_MAX_LRU_CACHE_SIZE,
            deployment_max_lru_cache_sizes: Box::default(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = environment_size(lookup)?;
        let path = "litellm_settings.DEFAULT_MAX_LRU_CACHE_SIZE".to_owned();
        let global = resolve_number(
            self.litellm_settings.default_max_lru_cache_size,
            &path,
            lookup,
            parse_size,
        )?
        .or(environment)
        .unwrap_or(DEFAULT_MAX_LRU_CACHE_SIZE);
        let mut deployment_max_lru_cache_sizes = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let deployment_path =
                format!("model_list.{index}.litellm_params.DEFAULT_MAX_LRU_CACHE_SIZE");
            let configured = resolve_number(
                deployment.litellm_params.default_max_lru_cache_size,
                &deployment_path,
                lookup,
                parse_size,
            )?;
            deployment_max_lru_cache_sizes.push(configured.unwrap_or(global));
        }
        Ok(Settings {
            default_max_lru_cache_size: global,
            deployment_max_lru_cache_sizes: deployment_max_lru_cache_sizes.into_boxed_slice(),
        })
    }
}

// an empty environment value counts as unset, as docs/compatibility.md states
fn environment_size<L>(lookup: &L) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = lookup
        .lookup("DEFAULT_MAX_LRU_CACHE_SIZE")
        .map(|text| betterlite_env::number::<i64>("DEFAULT_MAX_LRU_CACHE_SIZE", &text))
        .transpose()?
        .flatten();
    Ok(value)
}

fn parse_size(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
