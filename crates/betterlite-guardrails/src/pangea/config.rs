use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::PangeaGuardrailError;

pub const DEFAULT_API_BASE: &str = "https://ai-guard.aws.us.pangea.cloud";
pub const API_KEY_VARIABLE: &str = "PANGEA_API_KEY";
pub const API_BASE_VARIABLE: &str = "PANGEA_API_BASE";

// The UI schema LiteLLM returns from get_config_model; its optional_params group is not what initialize_guardrail reads.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PangeaGuardrailConfigModel {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub optional_params: Option<PangeaOptionalParams>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PangeaOptionalParams {
    pub pangea_input_recipe: Option<String>,
    pub pangea_output_recipe: Option<String>,
}

impl GuardrailConfigModelIdentity for PangeaGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Pangea Guardrail"
    }
}

// The litellm_params LiteLLM's initialize_guardrail reads; the recipes are flat keys there (pangea/__init__.py L17-L18).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PangeaGuardrailParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub pangea_input_recipe: Option<String>,
    pub pangea_output_recipe: Option<String>,
}

#[derive(Debug)]
pub struct PangeaSettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub input_recipe: Option<String>,
    pub output_recipe: Option<String>,
}

impl PangeaGuardrailParams {
    pub fn resolve<L>(self, lookup: &L) -> Result<PangeaSettings, PangeaGuardrailError>
    where
        L: EnvironmentLookup,
    {
        let env_key = lookup
            .lookup(API_KEY_VARIABLE)
            .filter(|value| !value.is_empty())
            .map(SecretString::from);
        let api_key = self
            .api_key
            .or(env_key)
            .ok_or(PangeaGuardrailError::MissingSecrets)?;
        let env_base = lookup
            .lookup(API_BASE_VARIABLE)
            .filter(|value| !value.is_empty());
        let api_base = self
            .api_base
            .or(env_base)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(PangeaSettings {
            api_key,
            api_base,
            input_recipe: self.pangea_input_recipe,
            output_recipe: self.pangea_output_recipe,
        })
    }
}
