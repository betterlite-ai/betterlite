mod tests;

use serde::Deserialize;
use serde_json::Number;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// LiteLLM's AmazonConverseConfig annotates every field None and stores only the values a caller passes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConverseOptionValues {
    pub max_tokens: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub top_k: Option<i64>,
}

impl ConverseOptionValues {
    // A deployment's litellm_params win field by field over litellm_settings.
    #[must_use]
    pub fn merged_with(&self, deployment: &Self) -> Self {
        Self {
            max_tokens: deployment.max_tokens.or(self.max_tokens),
            stop_sequences: deployment
                .stop_sequences
                .clone()
                .or_else(|| self.stop_sequences.clone()),
            temperature: deployment
                .temperature
                .clone()
                .or_else(|| self.temperature.clone()),
            top_p: deployment.top_p.clone().or_else(|| self.top_p.clone()),
            top_k: deployment.top_k.or(self.top_k),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Vec<ConverseOptionValues>,
    pub litellm_settings: ConverseOptionValues,
}

impl Settings {
    #[must_use]
    pub fn resolve(&self, index: usize) -> ConverseOptionValues {
        self.deployments.get(index).map_or_else(
            || self.litellm_settings.clone(),
            |deployment| self.litellm_settings.merged_with(deployment),
        )
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(rename = "maxTokens")]
    max_tokens: Option<i64>,
    #[serde(rename = "stopSequences")]
    stop_sequences: Option<Vec<String>>,
    temperature: Option<Number>,
    #[serde(rename = "topP")]
    top_p: Option<Number>,
    #[serde(rename = "topK")]
    top_k: Option<i64>,
}

impl RawParams {
    fn into_values(self) -> ConverseOptionValues {
        ConverseOptionValues {
            max_tokens: self.max_tokens,
            stop_sequences: self.stop_sequences,
            temperature: self.temperature,
            top_p: self.top_p,
            top_k: self.top_k,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            deployments: self
                .model_list
                .into_iter()
                .map(|deployment| deployment.litellm_params.into_values())
                .collect(),
            litellm_settings: self.litellm_settings.into_values(),
        })
    }
}
