use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
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
    quality_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    litellm_routing_preferences: Option<RawPreferences>,
    input_cost_per_token: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
struct RawRouterConfig {
    #[serde(default)]
    available_models: Vec<String>,
    default_model: Option<String>,
    complexity_to_quality: Option<BTreeMap<String, RawNumber>>,
    // LiteLLM's config model allows extra fields, so an unknown key is kept and reported, not refused
    #[serde(flatten)]
    extras: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
struct RawPreferences {
    quality_tier: RawNumber,
    #[serde(default)]
    keywords: Vec<String>,
    order: Option<RawNumber>,
    #[serde(flatten)]
    extras: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterConfig {
    pub available_models: Vec<String>,
    pub default_model: Option<String>,
    // None when the file omits the key, so the pinned default table applies
    pub complexity_to_quality: Option<BTreeMap<String, i64>>,
    pub extras: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    pub quality_tier: i64,
    pub keywords: Vec<String>,
    pub order: Option<i64>,
    pub extras: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deployment {
    pub model_name: String,
    pub config: Option<RouterConfig>,
    pub preferences: Option<Preferences>,
    pub input_cost_per_token: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; every entry is kept, since available_models may name any of them
    pub deployments: Vec<Deployment>,
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
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Deployment, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}");
        let config = self
            .litellm_params
            .quality_router_config
            .map(|config| {
                config.resolve(
                    &format!("{prefix}.litellm_params.quality_router_config"),
                    lookup,
                )
            })
            .transpose()?;
        let preferences = self
            .model_info
            .litellm_routing_preferences
            .map(|preferences| {
                preferences.resolve(
                    &format!("{prefix}.model_info.litellm_routing_preferences"),
                    lookup,
                )
            })
            .transpose()?;
        let cost_path = format!("{prefix}.model_info.input_cost_per_token");
        let input_cost_per_token =
            resolve::money(self.model_info.input_cost_per_token, &cost_path, lookup)?;
        Ok(Deployment {
            model_name: self.model_name,
            config,
            preferences,
            input_cost_per_token,
        })
    }
}
