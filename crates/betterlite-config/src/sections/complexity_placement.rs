use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const RETURN_RAW_MODEL_NAME_KEY: &str = "return_raw_model_name";
const MAX_TOKENS_FROM_TIER_MODEL_KEY: &str = "max_tokens_from_tier_model";
const DEPLOYMENT_AFFINITY_KEY: &str = "deployment_affinity";
const SESSION_AFFINITY_KEY: &str = "session_affinity";
const ENV_MARKER: &str = "os.environ/";

pub const DEFAULT_RETURN_RAW_MODEL_NAME: bool = false;
pub const DEFAULT_MAX_TOKENS_FROM_TIER: bool = true;

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
    #[serde(default, rename = "return_raw_model_name")]
    raw_model_name: Option<RawFlag>,
    #[serde(default, rename = "max_tokens_from_tier_model")]
    tier_model_ceiling: Option<RawFlag>,
    #[serde(default)]
    deployment_affinity: Option<RawFlag>,
    #[serde(default)]
    session_affinity: Option<RawFlag>,
    #[serde(default)]
    classification_mode: Option<ClassificationMode>,
    #[serde(default)]
    plugins: Option<Vec<JsonValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
}

// ComplexityRouterConfig.classification_mode's declared values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClassificationMode {
    EveryRequest,
    UserTurn,
}

// ComplexityRouter._uses_deployment_pin: whether the router stamps the session-affinity marker before routing
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeploymentPin {
    #[default]
    Unused,
    InUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub return_raw_model_name: bool,
    pub max_tokens_from_tier_model: bool,
    pub deployment_pin: DeploymentPin,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            return_raw_model_name: DEFAULT_RETURN_RAW_MODEL_NAME,
            max_tokens_from_tier_model: DEFAULT_MAX_TOKENS_FROM_TIER,
            deployment_pin: DeploymentPin::Unused,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no placement settings
    pub deployments: Vec<Option<Config>>,
}

impl Settings {
    // ComplexityRouter._uses_deployment_pin, over every router the config registers
    #[must_use]
    pub fn stamps_session_marker(&self) -> bool {
        self.deployments
            .iter()
            .flatten()
            .any(|config| config.deployment_pin == DeploymentPin::InUse)
    }
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
        let deployment_affinity = flag(
            self.deployment_affinity,
            &format!("{prefix}.{DEPLOYMENT_AFFINITY_KEY}"),
            lookup,
        )?
        .unwrap_or(false);
        let session_affinity = flag(
            self.session_affinity,
            &format!("{prefix}.{SESSION_AFFINITY_KEY}"),
            lookup,
        )?
        .unwrap_or(false);
        let user_turn = self.classification_mode == Some(ClassificationMode::UserTurn);
        // LiteLLM reads a configured plugin list by truthiness, and plugins replace the pin machinery
        let plugins = self.plugins.is_some_and(|plugins| !plugins.is_empty());
        let pin = !plugins && (deployment_affinity || session_affinity || user_turn);
        Ok(Config {
            return_raw_model_name: flag(
                self.raw_model_name,
                &format!("{prefix}.{RETURN_RAW_MODEL_NAME_KEY}"),
                lookup,
            )?
            .unwrap_or(DEFAULT_RETURN_RAW_MODEL_NAME),
            max_tokens_from_tier_model: flag(
                self.tier_model_ceiling,
                &format!("{prefix}.{MAX_TOKENS_FROM_TIER_MODEL_KEY}"),
                lookup,
            )?
            .unwrap_or(DEFAULT_MAX_TOKENS_FROM_TIER),
            deployment_pin: if pin {
                DeploymentPin::InUse
            } else {
                DeploymentPin::Unused
            },
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
        Some(RawFlag::Literal(flag)) => Ok(Some(flag)),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?)
        }
    }
}
