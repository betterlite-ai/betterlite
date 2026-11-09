use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const EMBEDDING_MODEL_PATH: &str = "litellm_settings.agent_search_embedding_model";
const PROBE_TIMEOUT_SECONDS: f64 = 5.0;
const GATHER_TIMEOUT_SECONDS: f64 = 30.0;
const PROBE_TIMEOUT_VARIABLE: &str = "LITELLM_AGENT_HEALTH_CHECK_TIMEOUT";
const GATHER_TIMEOUT_VARIABLE: &str = "LITELLM_AGENT_HEALTH_CHECK_GATHER_TIMEOUT";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    agent_search_embedding_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub embedding_model: Option<String>,
    pub health_check_timeout_seconds: f64,
    pub health_gather_timeout_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            embedding_model: None,
            health_check_timeout_seconds: PROBE_TIMEOUT_SECONDS,
            health_gather_timeout_seconds: GATHER_TIMEOUT_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM's get_secret returns None for an unset os.environ/ variable, so the model stays unset
        let embedding_model = self
            .litellm_settings
            .agent_search_embedding_model
            .and_then(|model| resolve_string(model, EMBEDDING_MODEL_PATH, lookup).ok());
        Ok(Settings {
            embedding_model,
            health_check_timeout_seconds: seconds_from_environment(lookup, PROBE_TIMEOUT_VARIABLE)?
                .unwrap_or(PROBE_TIMEOUT_SECONDS),
            health_gather_timeout_seconds: seconds_from_environment(
                lookup,
                GATHER_TIMEOUT_VARIABLE,
            )?
            .unwrap_or(GATHER_TIMEOUT_SECONDS),
        })
    }
}

// LiteLLM calls float() on both variables, so a non-numeric value stops startup and names the variable
fn seconds_from_environment<L>(lookup: &L, variable: &str) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(variable) else {
        return Ok(None);
    };
    Ok(betterlite_env::number::<f64>(variable, &text)?)
}
