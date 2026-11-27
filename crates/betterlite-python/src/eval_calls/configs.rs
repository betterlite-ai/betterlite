use std::collections::BTreeMap;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use betterlite_core::api::eval_create_list::config::{EvalsConfigLookup, EvalsConfigs};
use betterlite_core::api::eval_http::exchange::EvalConfig;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use secrecy::{ExposeSecret, SecretString};

use super::convert::ModuleSettings;

// The one evals provider betterlite serves, held for both core lookups: the borrowed one create and list use
// and the owned one the lifecycle service takes.
pub(super) struct EvalConfigs {
    shared: EvalsConfigs,
}

impl EvalConfigs {
    pub(super) fn new(settings: &ModuleSettings, environment: &BTreeMap<String, String>) -> Self {
        Self {
            shared: EvalsConfigs {
                openai: OpenAiEvalsConfig {
                    api_key: copy_secret(settings.api_key.as_ref()),
                    openai_key: copy_secret(settings.openai_key.as_ref()),
                    env_api_key: environment.get("OPENAI_API_KEY").map(SecretString::from),
                },
            },
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
        (provider == DEFAULT_EVALS_PROVIDER).then(|| copy(&self.shared.openai))
    }
}

fn copy(config: &OpenAiEvalsConfig) -> OpenAiEvalsConfig {
    OpenAiEvalsConfig {
        api_key: copy_secret(config.api_key.as_ref()),
        openai_key: copy_secret(config.openai_key.as_ref()),
        env_api_key: copy_secret(config.env_api_key.as_ref()),
    }
}

// SecretBox is not Clone, so the exposed text goes back through SecretString's own wrapper
fn copy_secret(secret: Option<&SecretString>) -> Option<SecretString> {
    secret.map(|value| SecretString::from(value.expose_secret().to_owned()))
}
