use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};

use crate::model_group_info::sources::{
    ProviderAnswer, ProviderFailure, ProviderRequest, ProviderResolver,
};

pub struct ContextProviderResolver<'resolution> {
    pub resolution: &'resolution ResolutionContext<'resolution>,
}

impl ProviderResolver for ContextProviderResolver<'_> {
    type Error = ResolutionError;

    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure<ResolutionError>> {
        let lookup = ResolutionRequest {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            api_base: None,
            api_key: None,
        };
        match resolve(&lookup, self.resolution) {
            Ok(found) => Ok(ProviderAnswer {
                litellm_model: found.model,
                provider: found.provider.slug().to_owned(),
            }),
            Err(ResolutionError::MissingProvider { model }) => {
                Err(ProviderFailure::BadRequest(model))
            }
            Err(source) => Err(ProviderFailure::Other(source)),
        }
    }
}
