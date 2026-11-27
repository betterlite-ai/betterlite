use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;

use crate::api::eval_http::exchange::EvalConfig;

// ProviderConfigManager.get_provider_evals_api_config: the config a selected provider resolves to
pub trait EvalsConfigLookup: Send + Sync {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>>;
}

// the configs B3420's selector can name, built by the caller that holds the credentials
pub struct EvalsConfigs {
    pub openai: OpenAiEvalsConfig,
}

impl EvalsConfigLookup for EvalsConfigs {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>> {
        match tag {
            EvalsConfigTag::OpenAI => Some(&self.openai),
        }
    }
}
