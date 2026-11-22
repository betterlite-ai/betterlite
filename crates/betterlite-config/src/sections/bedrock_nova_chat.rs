mod tests;

use betterlite_models::decisions::Json as JsonData;
use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_secret};
use crate::sections::Section;

pub const DEFAULT_API_BASE: &str = "https://api.nova.amazon.com/v1";
const API_BASE_VARIABLE: &str = "AMAZON_NOVA_API_BASE";
const API_KEY_VARIABLE: &str = "AMAZON_NOVA_API_KEY";

// AmazonNovaChatConfig's six constructor fields, then the metadata class field the constructor cannot set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NovaChatValues {
    pub max_completion_tokens: Option<i64>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<i64>,
    pub top_p: Option<i64>,
    pub tools: Option<Vec<JsonData>>,
    pub reasoning_effort: Option<Vec<JsonData>>,
    pub metadata: Option<i64>,
}

impl NovaChatValues {
    #[must_use]
    pub fn merged_with(&self, deployment: &Self) -> Self {
        Self {
            max_completion_tokens: deployment
                .max_completion_tokens
                .or(self.max_completion_tokens),
            max_tokens: deployment.max_tokens.or(self.max_tokens),
            temperature: deployment.temperature.or(self.temperature),
            top_p: deployment.top_p.or(self.top_p),
            tools: deployment.tools.clone().or_else(|| self.tools.clone()),
            reasoning_effort: deployment
                .reasoning_effort
                .clone()
                .or_else(|| self.reasoning_effort.clone()),
            metadata: deployment.metadata.or(self.metadata),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub deployments: Vec<NovaChatValues>,
    pub litellm_settings: NovaChatValues,
    pub amazon_nova_api_key: Option<SecretString>,
    pub api_base_environment: Option<String>,
    pub api_key_environment: Option<SecretString>,
}

impl Settings {
    #[must_use]
    pub fn resolve(&self, index: usize) -> NovaChatValues {
        self.deployments.get(index).map_or_else(
            || self.litellm_settings.clone(),
            |deployment| self.litellm_settings.merged_with(deployment),
        )
    }

    // The explicit api_base and api_key are the deployment's own parameters; these are the fallbacks.
    #[must_use]
    pub fn api_base(&self) -> String {
        self.api_base_environment
            .clone()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
    }

    #[must_use]
    pub fn api_key(&self) -> Option<SecretString> {
        self.amazon_nova_api_key
            .clone()
            .or_else(|| self.api_key_environment.clone())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    model_list: Vec<RawDeployment>,
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawDeployment {
    litellm_params: RawParameters,
}

// The class fields sit under their provider name, apart from the request defaults litellm_params also holds.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawParameters {
    amazon_nova: RawValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSettings {
    amazon_nova_api_key: Option<SecretString>,
    amazon_nova: RawValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawValues {
    max_completion_tokens: Option<RawNumber>,
    max_tokens: Option<RawNumber>,
    temperature: Option<RawNumber>,
    top_p: Option<RawNumber>,
    tools: Option<Vec<JsonData>>,
    reasoning_effort: Option<Vec<JsonData>>,
    metadata: Option<RawNumber>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params.amazon_nova");
            deployments.push(option_values(
                deployment.litellm_params.amazon_nova,
                &prefix,
                lookup,
            )?);
        }
        Ok(Settings {
            deployments,
            litellm_settings: option_values(
                self.litellm_settings.amazon_nova,
                "litellm_settings.amazon_nova",
                lookup,
            )?,
            amazon_nova_api_key: resolve_secret(
                self.litellm_settings.amazon_nova_api_key,
                "litellm_settings.amazon_nova_api_key",
                lookup,
            )?,
            api_base_environment: lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|value| !value.is_empty()),
            api_key_environment: lookup
                .lookup(API_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}

fn option_values<L>(raw: RawValues, prefix: &str, lookup: &L) -> Result<NovaChatValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("{prefix}.{key}");
    Ok(NovaChatValues {
        max_completion_tokens: resolve_number(
            raw.max_completion_tokens,
            &path("max_completion_tokens"),
            lookup,
            integer,
        )?,
        max_tokens: resolve_number(raw.max_tokens, &path("max_tokens"), lookup, integer)?,
        temperature: resolve_number(raw.temperature, &path("temperature"), lookup, integer)?,
        top_p: resolve_number(raw.top_p, &path("top_p"), lookup, integer)?,
        tools: raw.tools,
        reasoning_effort: raw.reasoning_effort,
        metadata: resolve_number(raw.metadata, &path("metadata"), lookup, integer)?,
    })
}

fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse().map_err(|source| ConfigError::InvalidInteger {
        path: path.to_owned(),
        source,
    })
}
