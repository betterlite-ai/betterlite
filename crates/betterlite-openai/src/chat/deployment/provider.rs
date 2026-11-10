use betterlite_config::types::LiteLlmParams;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_a::error::ProviderDefaultsError as DefaultsAError;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest,
    resolve as resolve_model,
};

use super::Provider;

// Only the OpenAI adapter is built, so every other resolved identity stays a clear refusal.
pub(super) fn resolved_provider(
    litellm_model: &str,
    custom_llm_provider: Option<&str>,
    params: &LiteLlmParams,
    context: &ResolutionContext<'_>,
) -> Result<Provider, ResolutionError> {
    let request = ResolutionRequest {
        model: litellm_model,
        custom_llm_provider,
        api_base: params.api_base.as_deref(),
        api_key: params.api_key.as_ref(),
    };
    let openai = match resolve_model(&request, context) {
        Ok(resolved) => matches!(
            resolved.provider,
            ProviderIdentity::Known(ProviderId::Openai)
        )
        .then_some(resolved.model),
        // The Mantle endpoint is an input betterlite does not build, so only its missing adapter refuses the call.
        Err(ResolutionError::DefaultsA {
            source: DefaultsAError::MissingMantleEndpoint,
        }) => None,
        Err(error) => return Err(error),
    };
    Ok(openai.map_or_else(
        || Provider::Unsupported {
            model: litellm_model.to_owned(),
        },
        |model| Provider::OpenAi { model },
    ))
}
