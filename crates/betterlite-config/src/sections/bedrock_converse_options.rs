mod tests;

use serde::Deserialize;
use serde_json::Number;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConverseOptionValues {
    pub max_tokens: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub top_k: Option<i64>,
}

impl ConverseOptionValues {
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
    litellm_settings: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawValues {
    #[serde(rename = "maxTokens")]
    max_tokens: Option<RawNumber>,
    #[serde(rename = "stopSequences")]
    stop_sequences: Option<Vec<String>>,
    temperature: Option<RawNumber>,
    #[serde(rename = "topP")]
    top_p: Option<RawNumber>,
    #[serde(rename = "topK")]
    top_k: Option<RawNumber>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            deployments.push(option_values(deployment.litellm_params, &prefix, lookup)?);
        }
        Ok(Settings {
            deployments,
            litellm_settings: option_values(self.litellm_settings, "litellm_settings", lookup)?,
        })
    }
}

fn option_values<L>(
    raw: RawValues,
    prefix: &str,
    lookup: &L,
) -> Result<ConverseOptionValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("{prefix}.{key}");
    Ok(ConverseOptionValues {
        max_tokens: resolve_number(raw.max_tokens, &path("maxTokens"), lookup, integer)?,
        stop_sequences: raw.stop_sequences,
        temperature: resolve_number(raw.temperature, &path("temperature"), lookup, number)?,
        top_p: resolve_number(raw.top_p, &path("topP"), lookup, number)?,
        top_k: resolve_number(raw.top_k, &path("topK"), lookup, integer)?,
    })
}

fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse().map_err(|source| ConfigError::InvalidInteger {
        path: path.to_owned(),
        source,
    })
}

// Keep the integer or decimal spelling the YAML gave, as LiteLLM's Python value does.
fn number(text: &str, path: &str) -> Result<Number, ConfigError> {
    let _validated = parse_float(text, path)?;
    text.parse().map_err(|_source| ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    })
}
