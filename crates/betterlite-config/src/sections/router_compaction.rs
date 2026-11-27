use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const FIELD_KEY: &str = "context_compaction";
const ENV_MARKER: &str = "os.environ/";

pub const DEFAULT_TRIGGER_RATIO: f64 = 0.9;
pub const DEFAULT_MAX_TOKENS: i64 = 4096;
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;
pub const MIN_MAX_TOKENS: i64 = 512;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    complexity_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouterConfig {
    // an absent key builds the default config, while false or null turns compaction off
    #[serde(default, rename = "context_compaction")]
    context_compaction: RawContextCompaction,
}

// Missing comes from the field default, so an absent key differs from an explicit null
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawContextCompaction {
    Flag(bool),
    Object(RawContextCompactionConfig),
    Null(()),
    #[default]
    Missing,
}

#[derive(Debug, Default, Deserialize)]
struct RawContextCompactionConfig {
    model: Option<String>,
    trigger_ratio: Option<RawNumber>,
    max_tokens: Option<RawNumber>,
    timeout_seconds: Option<RawNumber>,
    // LiteLLM's config model is extra="forbid", so a key left over is refused with its path
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContextCompaction {
    Off,
    On(ContextCompactionConfig),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextCompactionConfig {
    pub model: Option<String>,
    pub trigger_ratio: f64,
    pub max_tokens: i64,
    pub timeout_seconds: f64,
}

impl Default for ContextCompactionConfig {
    fn default() -> Self {
        Self {
            model: None,
            trigger_ratio: DEFAULT_TRIGGER_RATIO,
            max_tokens: DEFAULT_MAX_TOKENS,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

// index-aligned with model_list; None when the deployment configures no complexity router at all
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub deployments: Vec<Option<ContextCompaction>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let configured = deployment.litellm_params.complexity_router_config;
            deployments.push(
                configured
                    .map(|raw| raw.resolve(index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}

impl RawRouterConfig {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<ContextCompaction, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = format!("model_list.{index}.{PATH_ROOT}.{FIELD_KEY}");
        match self.context_compaction {
            RawContextCompaction::Flag(false) | RawContextCompaction::Null(()) => {
                Ok(ContextCompaction::Off)
            }
            RawContextCompaction::Flag(true) => {
                Err(ConfigError::InvalidScalar { path, value: true })
            }
            RawContextCompaction::Object(raw) => {
                raw.resolve(&path, lookup).map(ContextCompaction::On)
            }
            RawContextCompaction::Missing => {
                Ok(ContextCompaction::On(ContextCompactionConfig::default()))
            }
        }
    }
}

impl RawContextCompactionConfig {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<ContextCompactionConfig, ConfigError>
    where
        L: EnvironmentLookup,
    {
        if !self.extra.is_empty() {
            let paths = self
                .extra
                .into_keys()
                .map(|key| format!("{path}.{key}"))
                .collect();
            return Err(ConfigError::Unsupported { paths });
        }
        let trigger_path = format!("{path}.trigger_ratio");
        let trigger_ratio = resolve_number(self.trigger_ratio, &trigger_path, lookup, parse_float)?
            .unwrap_or(DEFAULT_TRIGGER_RATIO);
        // pydantic declares trigger_ratio gt=0 lt=1, open at both ends
        if !(0.0_f64..1.0_f64).contains(&trigger_ratio) {
            return Err(invalid_scalar(&trigger_path));
        }
        let tokens_path = format!("{path}.max_tokens");
        let max_tokens = resolve_number(self.max_tokens, &tokens_path, lookup, parse_count)?
            .unwrap_or(DEFAULT_MAX_TOKENS);
        // pydantic declares max_tokens ge=512 and accepts 512 itself
        if max_tokens < MIN_MAX_TOKENS {
            return Err(invalid_scalar(&tokens_path));
        }
        let timeout_path = format!("{path}.timeout_seconds");
        let timeout_seconds =
            resolve_number(self.timeout_seconds, &timeout_path, lookup, parse_float)?
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        // pydantic declares timeout_seconds gt=0
        if timeout_seconds <= 0.0_f64 {
            return Err(invalid_scalar(&timeout_path));
        }
        Ok(ContextCompactionConfig {
            model: model_name(self.model, &format!("{path}.model"), lookup)?,
            trigger_ratio,
            max_tokens,
            timeout_seconds,
        })
    }
}

// an os.environ/ reference to an empty variable counts as unset, and a written empty model is refused
fn model_name<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let reference = text.starts_with(ENV_MARKER);
    let resolved = resolve_string(text, path, lookup)?;
    if reference && resolved.is_empty() {
        return Ok(None);
    }
    if resolved.is_empty() {
        return Err(invalid_scalar(path));
    }
    Ok(Some(resolved))
}

// pydantic declares max_tokens as an int, and a fractional value reaches the loader as text
fn parse_count(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn invalid_scalar(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
