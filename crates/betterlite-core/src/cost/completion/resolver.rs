use betterlite_models::resolution::inferred::inferred_provider;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};

use super::lookup::CatalogLookup;
use crate::cost::context::ProviderInference;
use crate::cost::special::policy::ModelResolver;

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no provider can be inferred from the model name")]
pub struct NoProvider;

impl ModelResolver for CatalogLookup {
    type Failure = ResolutionError;

    fn resolve_model(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        let request = ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: None,
            api_key: None,
        };
        resolve(&request, &self.resolution())
    }
}

impl ProviderInference for CatalogLookup {
    type Failure = NoProvider;

    fn inferred_provider(&self, model: &str) -> Result<ProviderIdentity, NoProvider> {
        inferred_provider(model, &self.resolution()).ok_or(NoProvider)
    }
}
