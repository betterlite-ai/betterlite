use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_a::error::ProviderDefaultsError as DefaultsAError;
use crate::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest as RequestA, ResolvedProvider as ResolvedA,
};
use crate::provider_defaults_a::resolve::resolve_provider_defaults as resolve_a;
use crate::provider_defaults_b::error::ProviderDefaultsError as DefaultsBError;
use crate::provider_defaults_b::model::{
    ProviderDefaultsRequest as RequestB, ResolvedProvider as ResolvedB,
};
use crate::provider_defaults_b::resolve::resolve_provider_defaults as resolve_b;

use super::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel,
};

// LiteLLM's _get_openai_compatible_provider_info: the two default tables split one if/elif chain.
pub(super) fn resolve(
    identity: ProviderIdentity,
    model: String,
    request: &ResolutionRequest<'_>,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedModel, ResolutionError> {
    let provider = match &identity {
        ProviderIdentity::Known(provider) => *provider,
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => {
            return Ok(super::unchanged(model, identity, request));
        }
    };
    let first = RequestA {
        provider: DefaultsProvider::Known(provider),
        model: model.clone(),
        api_base: request.api_base.map(str::to_owned),
        api_key: request.api_key.cloned(),
    };
    match resolve_a(
        first,
        context.environment,
        &context.registry,
        &context.policies,
    ) {
        Ok(resolved) => return Ok(from_a(resolved)),
        Err(DefaultsAError::UnknownProvider { .. }) => {}
        Err(source) => return Err(ResolutionError::DefaultsA { source }),
    }
    // A provider that neither table knows keeps the caller's base and key (the final api_key fallback, line 934).
    let second = RequestB {
        provider,
        model: model.clone(),
        api_base: request.api_base.map(str::to_owned),
        api_key: request.api_key.cloned(),
    };
    match resolve_b(second, &context.globals, context.environment) {
        Ok(resolved) => Ok(from_b(provider, resolved)),
        Err(DefaultsBError::UnknownProvider { .. }) => Ok(ResolvedModel {
            api_key: request.api_key.cloned(),
            ..super::unchanged(model, ProviderIdentity::Known(provider), request)
        }),
        Err(source) => Err(ResolutionError::DefaultsB { source }),
    }
}

fn from_a(resolved: ResolvedA) -> ResolvedModel {
    ResolvedModel {
        model: resolved.model,
        provider: match resolved.provider {
            DefaultsProvider::Known(provider) => ProviderIdentity::Known(provider),
            DefaultsProvider::Anyscale => ProviderIdentity::Anyscale,
        },
        api_base: resolved.api_base,
        api_key: resolved.api_key,
    }
}

fn from_b(provider: ProviderId, resolved: ResolvedB) -> ResolvedModel {
    ResolvedModel {
        model: resolved.model,
        provider: ProviderIdentity::Known(provider),
        api_base: resolved.api_base,
        api_key: resolved.api_key,
    }
}
