use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod resolve;
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    model_name: String,
    #[serde(default)]
    litellm_params: RawParams,
    #[serde(default)]
    model_info: RawInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    adaptive_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    adaptive_router_preferences: Option<RawPreferences>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouterConfig {
    available_models: Vec<String>,
    weights: Option<RawWeights>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWeights {
    quality: Option<RawNumber>,
    cost: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
struct RawPreferences {
    quality_tier: RawNumber,
    strengths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Weights {
    pub quality: Option<f64>,
    pub economy: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouterConfig {
    pub available_models: Vec<String>,
    pub weights: Option<Weights>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Preferences {
    pub quality_tier: i64,
    pub strengths: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deployment {
    pub model_name: String,
    pub config: Option<RouterConfig>,
    pub preferences: Option<Preferences>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment declares neither adaptive key
    pub deployments: Vec<Option<Deployment>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(deployment.resolve(index, lookup)?);
        }
        Ok(Settings { deployments })
    }
}

impl RawDeployment {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Option<Deployment>, ConfigError>
    where
        L: EnvironmentLookup,
    {
        if self.litellm_params.adaptive_router_config.is_none()
            && self.model_info.adaptive_router_preferences.is_none()
        {
            return Ok(None);
        }
        let prefix = format!("model_list.{index}");
        let config = self
            .litellm_params
            .adaptive_router_config
            .map(|config| {
                config.resolve(
                    &format!("{prefix}.litellm_params.adaptive_router_config"),
                    lookup,
                )
            })
            .transpose()?;
        let preferences = self
            .model_info
            .adaptive_router_preferences
            .map(|preferences| preferences.resolve(&prefix, lookup))
            .transpose()?;
        Ok(Some(Deployment {
            model_name: self.model_name,
            config,
            preferences,
        }))
    }
}

impl RawRouterConfig {
    fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<RouterConfig, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut available_models = Vec::with_capacity(self.available_models.len());
        for (position, model) in self.available_models.into_iter().enumerate() {
            let path = format!("{prefix}.available_models.{position}");
            available_models.push(resolve_string(model, &path, lookup)?);
        }
        let weights = match self.weights {
            Some(weights) => Some(weights.resolve(&format!("{prefix}.weights"), lookup)?),
            None => None,
        };
        Ok(RouterConfig {
            available_models,
            weights,
        })
    }
}
