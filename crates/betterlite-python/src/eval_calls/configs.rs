use std::collections::BTreeMap;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use betterlite_core::api::eval_create_list::config::{EvalsConfigLookup, EvalsConfigs};
use betterlite_core::api::eval_http::exchange::EvalConfig;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use secrecy::SecretString;

use super::convert::ModuleSettings;

// The one evals provider betterlite serves, held for both core lookups: borrowed for create and list, owned for lifecycle.
pub(super) struct EvalConfigs {
    shared: EvalsConfigs,
}

impl EvalConfigs {
    pub(super) fn new(settings: ModuleSettings, environment: &BTreeMap<String, String>) -> Self {
        let env_api_key = environment
            .get("OPENAI_API_KEY")
            .map(|value| SecretString::from(value.clone()));
        let openai = OpenAiEvalsConfig {
            api_key: settings.api_key,
            openai_key: settings.openai_key,
            env_api_key,
        };
        Self {
            shared: EvalsConfigs { openai },
        }
    }
}

impl EvalsConfigLookup for EvalConfigs {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>> {
        self.shared.evals_config(tag)
    }
}

impl EvalLifecycleConfigLookup for EvalConfigs {
    type Config = OpenAiEvalsConfig;

    fn eval_config(&self, provider: &str) -> Option<Self::Config> {
        // get_provider_evals_api_config builds a fresh OpenAIEvalsConfig for openai and None for every other name
        (provider == DEFAULT_EVALS_PROVIDER).then(|| OpenAiEvalsConfig {
            api_key: self.shared.openai.api_key.clone(),
            openai_key: self.shared.openai.openai_key.clone(),
            env_api_key: self.shared.openai.env_api_key.clone(),
        })
    }
}
