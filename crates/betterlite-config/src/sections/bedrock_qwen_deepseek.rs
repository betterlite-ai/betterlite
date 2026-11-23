mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

// AmazonQwen3Config's five constructor fields; each defaults to None on the class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QwenValues {
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub stop: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub deployments: Vec<QwenValues>,
    pub litellm_settings: QwenValues,
}

impl Settings {
    // Each constructor field the deployment set wins over the same field at litellm_settings.
    #[must_use]
    pub fn resolve(&self, index: usize) -> QwenValues {
        let Some(deployment) = self.deployments.get(index) else {
            return self.litellm_settings.clone();
        };
        QwenValues {
            max_tokens: deployment.max_tokens.or(self.litellm_settings.max_tokens),
            temperature: deployment.temperature.or(self.litellm_settings.temperature),
            top_p: deployment.top_p.or(self.litellm_settings.top_p),
            top_k: deployment.top_k.or(self.litellm_settings.top_k),
            stop: deployment
                .stop
                .clone()
                .or_else(|| self.litellm_settings.stop.clone()),
        }
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

// The class fields sit under their provider name, apart from the request parameters litellm_params also holds.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawParameters {
    amazon_qwen3: RawValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSettings {
    amazon_qwen3: RawValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawValues {
    max_tokens: Option<RawNumber>,
    temperature: Option<RawNumber>,
    top_p: Option<RawNumber>,
    top_k: Option<RawNumber>,
    stop: Option<Vec<String>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params.amazon_qwen3");
            deployments.push(values(
                deployment.litellm_params.amazon_qwen3,
                &prefix,
                lookup,
            )?);
        }
        Ok(Settings {
            deployments,
            litellm_settings: values(
                self.litellm_settings.amazon_qwen3,
                "litellm_settings.amazon_qwen3",
                lookup,
            )?,
        })
    }
}

fn values<L>(raw: RawValues, prefix: &str, lookup: &L) -> Result<QwenValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("{prefix}.{key}");
    Ok(QwenValues {
        max_tokens: resolve_number(raw.max_tokens, &path("max_tokens"), lookup, integer)?,
        temperature: resolve_number(raw.temperature, &path("temperature"), lookup, parse_float)?,
        top_p: resolve_number(raw.top_p, &path("top_p"), lookup, parse_float)?,
        top_k: resolve_number(raw.top_k, &path("top_k"), lookup, integer)?,
        stop: raw.stop,
    })
}

fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse().map_err(|source| ConfigError::InvalidInteger {
        path: path.to_owned(),
        source,
    })
}
