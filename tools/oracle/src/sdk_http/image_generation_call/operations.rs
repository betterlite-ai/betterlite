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
use secrecy::ExposeSecret as _;

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
        request: FoundryAuthRequest<'_>,
    ) -> Result<Headers, Box<RawException>> {
        request.api_key.map_or_else(
            || {
                Err(sends::plain(
                    "Missing Azure AI credentials - set an API key (`api_key` or AZURE_AI_API_KEY), or Entra ID / OAuth credentials",
                    "",
                    "azure_ai",
                ))
            },
            |key| {
                Ok(Headers::from([(
                    String::from("api-key"),
                    key.expose_secret().to_owned(),
                )]))
            },
        )
    }

    async fn send(
        &self,
        branch: Branch<'_>,
        _logging: &mut LoggingState,
    ) -> Result<BranchAnswer, Box<RawException>> {
        match branch {
            Branch::Azure(request) => sends::providers::azure(request, self.source).await,
            Branch::Generic(request) => sends::generic::send(request, self.source).await,
            Branch::Foundry(request) => sends::foundry::send(request, self.source).await,
            Branch::OpenAi(request) => sends::openai::send(request, self.source).await,
            Branch::Custom(request) => sends::custom::send(request, self.source).await,
            Branch::BlackForestLabs(request) => Err(sends::plain(
                "the black_forest_labs image handler is not wired into the oracle adapter",
                request.model,
                "black_forest_labs",
            )),
            Branch::Bedrock(request) => Err(sends::plain(
                "the bedrock image handler is not wired into the oracle adapter",
                request.model.unwrap_or_default(),
                "bedrock",
            )),
        }
    }
}
