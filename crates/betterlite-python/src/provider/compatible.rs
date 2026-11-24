use std::fmt::Display;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_a::error::ProviderDefaultsError as DefaultsAError;
use betterlite_models::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest as RequestA, ResolvedProvider as ResolvedA,
};
use betterlite_models::provider_defaults_a::resolve::resolve_provider_defaults as resolve_a;
use betterlite_models::provider_defaults_b::error::ProviderDefaultsError as DefaultsBError;
use betterlite_models::provider_defaults_b::model::ProviderDefaultsRequest as RequestB;
use betterlite_models::provider_defaults_b::resolve::resolve_provider_defaults as resolve_b;

use crate::provider::convert::{CompatibleRequest, LookupResult};
use crate::provider::error::ProviderError;
use crate::provider::snapshot::ProviderSnapshot;

// Ports _get_openai_compatible_provider_info: the provider's own branch resolves base and key, and a provider neither table knows keeps the caller's.
pub(crate) fn openai_compatible_info(
    snapshot: &ProviderSnapshot,
    request: &CompatibleRequest,
) -> Result<LookupResult, ProviderError> {
    let text = request.model.as_str();
    let Some((prefix, rest)) = text.split_once('/') else {
        return Err(wrap(&format!("{text} has no provider prefix"), text));
    };
    if let Ok(provider) = prefix.parse::<ProviderId>() {
        return known(snapshot, provider, rest, request);
    }
    // Only api_base inference can name anyscale, since LiteLLM's provider list has no member for it.
    if prefix == "anyscale" {
        let context = snapshot.context();
        let first = RequestA {
            provider: DefaultsProvider::Anyscale,
            model: rest.to_owned(),
            api_base: request.api_base.clone(),
            api_key: request.api_key.clone(),
        };
        return resolve_a(
            first,
            context.environment,
            &context.registry,
            &context.policies,
        )
        .map(from_a)
        .map_err(|source| wrap(&source, &request.model));
    }
    Ok(kept(prefix, rest, request))
}

fn known(
    snapshot: &ProviderSnapshot,
    provider: ProviderId,
    model: &str,
    request: &CompatibleRequest,
) -> Result<LookupResult, ProviderError> {
    let context = snapshot.context();
    let first = RequestA {
        provider: DefaultsProvider::Known(provider),
        model: model.to_owned(),
        api_base: request.api_base.clone(),
        api_key: request.api_key.clone(),
    };
    match resolve_a(
        first,
        context.environment,
        &context.registry,
        &context.policies,
    ) {
        Ok(resolved) => return Ok(from_a(resolved)),
        Err(DefaultsAError::UnknownProvider { .. }) => {}
        Err(source) => return Err(wrap(&source, &request.model)),
    }
    let fallback = kept(<&'static str>::from(provider), model, request);
    let second = RequestB {
        provider,
        model: model.to_owned(),
        api_base: request.api_base.clone(),
        api_key: request.api_key.clone(),
    };
    match resolve_b(second, &context.globals, context.environment) {
        Ok(resolved) => Ok(LookupResult {
            model: resolved.model,
            provider: <&'static str>::from(provider).to_owned(),
            api_key: resolved.api_key,
            api_base: resolved.api_base,
        }),
        Err(DefaultsBError::UnknownProvider { .. }) => Ok(fallback),
        Err(source) => Err(wrap(&source, &request.model)),
    }
}

// A provider no table answers for keeps the caller's base and key, the dynamic key winning when it is set.
fn kept(provider: &str, model: &str, request: &CompatibleRequest) -> LookupResult {
    LookupResult {
        model: model.to_owned(),
        provider: provider.to_owned(),
        api_key: request
            .dynamic_api_key
            .clone()
            .or_else(|| request.api_key.clone()),
        api_base: request.api_base.clone(),
    }
}

fn from_a(resolved: ResolvedA) -> LookupResult {
    LookupResult {
        model: resolved.model,
        provider: resolved.provider.slug().to_owned(),
        api_key: resolved.api_key,
        api_base: resolved.api_base,
    }
}

fn wrap(message: &dyn Display, model: &str) -> ProviderError {
    ProviderError::BadRequest {
        message: format!("GetLLMProvider Exception - {message}\n\noriginal model: {model}"),
        model: Some(model.to_owned()),
    }
}
