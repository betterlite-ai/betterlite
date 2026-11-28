use std::collections::BTreeMap;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_http::exchange::EvalConfig;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup;
use betterlite_core::api::eval_run_create_list::providers::EvalRunProviders;
use betterlite_core::api::eval_run_lifecycle::DEFAULT_PROVIDER;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use secrecy::SecretString;
use serde::Deserialize;

// The process settings one run session reads: no config file, so the environment is the whole input.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SessionWire {
    pub(super) environment: Option<BTreeMap<String, String>>,
    pub(super) settings: ModuleSettings,
}

// the betterlite module globals the OpenAI evals config falls back to, under the names it reads
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ModuleSettings {
    pub(super) api_key: Option<SecretString>,
    pub(super) openai_key: Option<SecretString>,
}

// The one evals provider betterlite serves, borrowed by the create and list service and copied for the lifecycle.
pub(super) struct EvalConfigs {
    openai: OpenAiEvalsConfig,
}

impl EvalConfigs {
    pub(super) fn new(settings: ModuleSettings, environment: &BTreeMap<String, String>) -> Self {
        let mut openai = OpenAiEvalsConfig {
            api_key: settings.api_key,
            openai_key: settings.openai_key,
            env_api_key: None,
        };
        if let Some(value) = environment.get("OPENAI_API_KEY") {
            openai.env_api_key = Some(SecretString::from(value.clone()));
        }
        Self { openai }
    }
}

impl EvalRunProviders for EvalConfigs {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>> {
        match tag {
            EvalsConfigTag::OpenAI => Some(&self.openai),
        }
    }
}

impl EvalLifecycleConfigLookup for EvalConfigs {
    type Config = OpenAiEvalsConfig;

    // get_provider_evals_api_config builds a fresh OpenAIEvalsConfig for openai and nothing for another name
    fn eval_config(&self, provider: &str) -> Option<Self::Config> {
        if provider != DEFAULT_PROVIDER {
            return None;
        }
        Some(OpenAiEvalsConfig {
            api_key: self.openai.api_key.clone(),
            openai_key: self.openai.openai_key.clone(),
            env_api_key: self.openai.env_api_key.clone(),
        })
    }
}
