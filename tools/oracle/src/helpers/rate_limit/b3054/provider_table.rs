use std::collections::BTreeMap;

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::resolution::ResolvedModel;

// the model spellings both sides resolve the same way: litellm.get_llm_provider sends these to their provider
pub(super) struct ProviderTable {
    providers: BTreeMap<String, String>,
}

impl ProviderTable {
    pub(super) fn litellm_defaults() -> Self {
        Self {
            providers: BTreeMap::from([(String::from("gpt-4o"), String::from("openai"))]),
        }
    }
}

impl ProviderResolution for ProviderTable {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        let Some(provider) = self.providers.get(model) else {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        };
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Custom(provider.clone()),
            api_base: None,
            api_key: None,
        })
    }
}
