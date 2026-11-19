use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const ENABLED_KEY: &str = "enable_context_window_escalation";
const BUFFER_KEY: &str = "context_window_escalation_buffer";
const ENV_MARKER: &str = "os.environ/";

pub const DEFAULT_ENABLED: bool = false;
pub const DEFAULT_BUFFER: f64 = 0.95;

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
    #[serde(default, rename = "enable_context_window_escalation")]
    enabled: Option<RawFlag>,
    #[serde(default, rename = "context_window_escalation_buffer")]
    buffer: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub buffer: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_ENABLED,
            buffer: DEFAULT_BUFFER,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no context-window gate
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
        let enabled_path = format!("{prefix}.{ENABLED_KEY}");
        // LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
        let enabled = match self.enabled {
            None => DEFAULT_ENABLED,
            Some(RawFlag::Literal(flag)) => flag,
            Some(RawFlag::Text(text)) => {
                let name = text
                    .strip_prefix(ENV_MARKER)
                    .unwrap_or(enabled_path.as_str())
                    .to_owned();
                let resolved = resolve_string(text, &enabled_path, lookup)?;
                betterlite_env::flag(&name, &resolved)?.unwrap_or(DEFAULT_ENABLED)
            }
        };
        let buffer = resolve_buffer(self.buffer, &prefix, lookup)?;
        Ok(Config { enabled, buffer })
    }
}

// a declared buffer is a number; only an os.environ reference may be text
fn resolve_buffer<L>(raw: Option<RawNumber>, prefix: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let buffer_path = format!("{prefix}.{BUFFER_KEY}");
    let Some(value) = raw else {
        return Ok(DEFAULT_BUFFER);
    };
    if let RawNumber::Text(text) = &value
        && !text.starts_with(ENV_MARKER)
    {
        return Err(ConfigError::InvalidScalar {
            path: buffer_path,
            value: false,
        });
    }
    // pydantic's gt=0, le=1 bound applies after an environment reference resolves, and a NaN fails it
    match resolve_number(Some(value), &buffer_path, lookup, parse_float)? {
        Some(buffer) if buffer.is_finite() && buffer > 0.0_f64 && buffer <= 1.0_f64 => Ok(buffer),
        Some(_) => Err(ConfigError::InvalidScalar {
            path: buffer_path,
            value: false,
        }),
        None => Ok(DEFAULT_BUFFER),
    }
}
