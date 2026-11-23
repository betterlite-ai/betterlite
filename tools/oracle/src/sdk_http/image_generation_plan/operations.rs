use std::sync::Arc;

use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_context::media_config_selection::image_generation::{
    ImageGenerationConfigTag, select_image_generation,
};
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_core::api::image_generation_call::operations::{
    Branch, BranchAnswer, EntraRequest, FoundryAuthRequest, Headers, ImageGenerationOperations,
};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::resolution::ProviderIdentity;

use super::sends;

pub(super) struct Operations<'source> {
    pub(super) source: &'source ProxiedClient,
}

impl ImageGenerationOperations for Operations<'_> {
    fn generation_config(
        &self,
        model: Option<&str>,
        provider: &ProviderIdentity,
    ) -> Option<ImageGenerationConfigTag> {
        match provider {
            ProviderIdentity::Known(id) => select_image_generation(model.unwrap_or_default(), *id),
            ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
        }
    }

    // B175's provider-config mappers are not built yet, so the parameter map stays the generic one
    fn options_mapper(
        &self,
        _config: &ImageGenerationConfigTag,
    ) -> Option<Box<dyn OptionalParamsMapper + '_>> {
        None
    }

    fn bedrock_mapper(&self) -> Option<&dyn OptionalParamsMapper> {
        None
    }

    fn entra_token_provider(
        &self,
        _request: EntraRequest<'_>,
    ) -> Option<Arc<dyn CallableTokenProvider>> {
        None
    }

    fn foundry_auth_headers(
        &self,
        _request: FoundryAuthRequest<'_>,
    ) -> Result<Headers, Box<RawException>> {
        Ok(Headers::new())
    }

    async fn send(
        &self,
        branch: Branch<'_>,
        _logging: &mut LoggingState,
    ) -> Result<BranchAnswer, Box<RawException>> {
        match branch {
            Branch::OpenAi(request) => sends::openai(request, self.source).await,
            Branch::Azure(request) => Err(sends::unsupported(request.model)),
            Branch::BlackForestLabs(request) => Err(sends::unsupported(Some(request.model))),
            Branch::Bedrock(request) => Err(sends::unsupported(request.model)),
            Branch::Custom(request) => Err(sends::unsupported(request.model)),
            Branch::Foundry(request) => Err(sends::unsupported(request.model)),
            Branch::Generic(request) => Err(sends::unsupported(request.model)),
        }
    }
}
