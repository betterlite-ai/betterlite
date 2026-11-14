use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

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

#[derive(Debug, Clone, Default, PartialEq)]
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
        let available_models = self
            .available_models
            .into_iter()
            .enumerate()
            .map(|(position, model)| {
                resolve_string(
                    model,
                    &format!("{prefix}.available_models.{position}"),
                    lookup,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let weights = self
            .weights
            .map(|weights| weights.resolve(&format!("{prefix}.weights"), lookup))
            .transpose()?;
        Ok(RouterConfig {
            available_models,
            weights,
        })
    }
}

impl RawWeights {
    fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Weights, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Weights {
            quality: weight(self.quality, &format!("{prefix}.quality"), lookup)?,
            economy: weight(self.cost, &format!("{prefix}.cost"), lookup)?,
        })
    }
}

impl RawPreferences {
    fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Preferences, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let tier_path = format!("{prefix}.model_info.adaptive_router_preferences.quality_tier");
        reject_quoted_number(&self.quality_tier, &tier_path)?;
        let quality_tier = resolve_number(Some(self.quality_tier), &tier_path, lookup, parse_tier)?
            .ok_or_else(|| invalid_scalar(&tier_path))?;
        let strengths_path = format!("{prefix}.model_info.adaptive_router_preferences.strengths");
        let strengths = self
            .strengths
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(position, strength)| {
                resolve_string(strength, &format!("{strengths_path}.{position}"), lookup)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Preferences {
            quality_tier,
            strengths,
        })
    }
}

fn weight<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(number) = &value {
        reject_quoted_number(number, path)?;
    }
    resolve_number(value, path, lookup, parse_float)
}

fn parse_tier(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

// a quoted number is a string to the declared float and int; only an os.environ reference may be text
fn reject_quoted_number(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    match value {
        RawNumber::Text(text) if !text.starts_with("os.environ/") => Err(invalid_scalar(path)),
        RawNumber::Integer(_)
        | RawNumber::Float(_)
        | RawNumber::Boolean(_)
        | RawNumber::Text(_) => Ok(()),
    }
}

fn invalid_scalar(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
