use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use serde::Deserialize;

use super::initialization::CodeInterpreterInterceptionLogger;

// CodeInterpreterInterceptionConfig from LiteLLM's types: every key optional, enabled defaulting to true
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CodeInterpreterInterceptionConfig {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    pub enabled_providers: Option<Vec<String>>,
    pub sandbox_tool_name: Option<String>,
}

const fn enabled_default() -> bool {
    true
}

impl Default for CodeInterpreterInterceptionConfig {
    fn default() -> Self {
        Self {
            enabled: enabled_default(),
            enabled_providers: None,
            sandbox_tool_name: None,
        }
    }
}

// The two places LiteLLM reads the interceptor's settings from: the litellm_settings entry wins over the callback-specific one
#[derive(Debug, Clone, Default)]
pub struct ProxyParameters {
    pub code_interpreter_interception_params: Option<CodeInterpreterInterceptionConfig>,
    pub callback_specific_params: Option<CodeInterpreterInterceptionConfig>,
}

#[must_use]
pub fn from_config_yaml<C>(
    config: &CodeInterpreterInterceptionConfig,
    catalog: Arc<ModelCatalog>,
) -> CodeInterpreterInterceptionLogger<C> {
    CodeInterpreterInterceptionLogger::new(
        config.enabled,
        config.enabled_providers.clone(),
        config.sandbox_tool_name.clone(),
        None,
        catalog,
    )
}

#[must_use]
pub fn initialize_from_proxy_config<C>(
    params: &ProxyParameters,
    catalog: Arc<ModelCatalog>,
) -> CodeInterpreterInterceptionLogger<C> {
    let Some(config) = params
        .code_interpreter_interception_params
        .as_ref()
        .or(params.callback_specific_params.as_ref())
    else {
        return from_config_yaml(&CodeInterpreterInterceptionConfig::default(), catalog);
    };
    from_config_yaml(config, catalog)
}

include!("configuration/tests/cases.rs");
