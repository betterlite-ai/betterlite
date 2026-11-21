use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const ENABLED_KEY: &str = "cache_aware_routing";
const OUTPUT_TOKENS_KEY: &str = "cache_aware_routing_output_tokens";
const TIMEOUT_MS_KEY: &str = "cache_aware_routing_timeout_ms";
const ENV_MARKER: &str = "os.environ/";

pub const DEFAULT_ENABLED: bool = false;
pub const DEFAULT_OUTPUT_TOKENS: u64 = 1024;
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

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
    #[serde(default, rename = "cache_aware_routing")]
    enabled: Option<RawFlag>,
    #[serde(default, rename = "cache_aware_routing_output_tokens")]
    output_tokens: Option<RawNumber>,
    #[serde(default, rename = "cache_aware_routing_timeout_ms")]
    timeout_ms: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub output_tokens: u64,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_ENABLED,
            output_tokens: DEFAULT_OUTPUT_TOKENS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no cache-aware settings
    pub deployments: Vec<Option<Config>>,
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
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}.{PATH_ROOT}");
        let timeout_path = format!("{prefix}.{TIMEOUT_MS_KEY}");
        let timeout_ms = resolve_number(self.timeout_ms, &timeout_path, lookup, parse_count)?
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        // cache_aware_routing_timeout_ms is declared gt=0: a zero budget expires before prediction runs
        if timeout_ms == 0 {
            return Err(invalid_scalar(&timeout_path));
        }
        Ok(Config {
            enabled: flag(self.enabled, &format!("{prefix}.{ENABLED_KEY}"), lookup)?
                .unwrap_or(DEFAULT_ENABLED),
            output_tokens: resolve_number(
                self.output_tokens,
                &format!("{prefix}.{OUTPUT_TOKENS_KEY}"),
                lookup,
                parse_count,
            )?
            .unwrap_or(DEFAULT_OUTPUT_TOKENS),
            timeout_ms,
        })
    }
}

// LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
fn flag<L>(raw: Option<RawFlag>, path: &str, lookup: &L) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        None => Ok(None),
        Some(RawFlag::Literal(value)) => Ok(Some(value)),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?)
        }
    }
}

// pydantic declares the two counts as int; ge=0 refuses negatives and gt=0 refuses a zero timeout
fn parse_count(text: &str, path: &str) -> Result<u64, ConfigError> {
    text.parse::<u64>()
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
