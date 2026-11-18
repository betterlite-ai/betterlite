use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
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
    #[serde(default)]
    enable_context_window_escalation: Option<RawFlag>,
    #[serde(default)]
    context_window_escalation_buffer: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
}

// the resolved config a deployment hands the context-window gate; both keys keep the spelling the file uses
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enable_context_window_escalation: bool,
    pub context_window_escalation_buffer: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable_context_window_escalation: DEFAULT_ENABLED,
            context_window_escalation_buffer: DEFAULT_BUFFER,
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
        let enabled = self
            .enable_context_window_escalation
            .map(|flag| {
                resolve_flag(
                    flag,
                    &format!("{prefix}.enable_context_window_escalation"),
                    lookup,
                )
            })
            .transpose()?
            .unwrap_or(DEFAULT_ENABLED);
        let buffer = self
            .context_window_escalation_buffer
            .map(|value| {
                resolve_buffer(
                    value,
                    &format!("{prefix}.context_window_escalation_buffer"),
                    lookup,
                )
            })
            .transpose()?
            .unwrap_or(DEFAULT_BUFFER);
        Ok(Config {
            enable_context_window_escalation: enabled,
            context_window_escalation_buffer: buffer,
        })
    }
}

fn resolve_flag<L>(value: RawFlag, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        RawFlag::Literal(flag) => Ok(flag),
        // LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
        RawFlag::Text(text) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(DEFAULT_ENABLED))
        }
    }
}

// a declared buffer is a number; only an os.environ reference may be text
fn resolve_buffer<L>(value: RawNumber, path: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let RawNumber::Text(text) = &value
        && !text.starts_with(ENV_MARKER)
    {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    }
    let resolved = resolve_number(Some(value), path, lookup, parse_float)?;
    // pydantic's gt=0, le=1 bound applies after an environment reference resolves, and a NaN fails it
    match resolved {
        Some(buffer) if buffer.is_finite() && buffer > 0.0 && buffer <= 1.0 => Ok(Some(buffer)),
        Some(_) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
        None => Ok(None),
    }
}
