use std::collections::{BTreeMap, BTreeSet};

use secrecy::SecretString;

use crate::catalog::ModelCatalog;
use crate::identity::explicit_prefix::{
    PrefixClassification, PrefixClassificationInput, classify_prefix,
};
use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::error::ProviderDefaultsError as DefaultsAError;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::error::ProviderDefaultsError as DefaultsBError;
use crate::provider_defaults_b::globals::GlobalCredentials;

mod defaults;
mod endpoint;
mod inference;
pub mod inferred;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderIdentity {
    Known(ProviderId),
    // LiteLLM's provider list has no anyscale member, so only api_base inference reaches it.
    Anyscale,
    Custom(String),
}

impl ProviderIdentity {
    #[must_use]
    pub fn slug(&self) -> &str {
        match self {
            Self::Known(provider) => <&'static str>::from(*provider),
            Self::Anyscale => "anyscale",
            Self::Custom(slug) => slug,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub model: String,
    pub provider: ProviderIdentity,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolutionError {
    #[error(
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={model}"
    )]
    MissingProvider { model: String },
    #[error(
        "api_base {api_base} matches the registered endpoint {endpoint}, which names no provider"
    )]
    EndpointWithoutProvider { endpoint: String, api_base: String },
    #[error("resolving the provider defaults failed: {source}")]
    DefaultsA {
        #[source]
        source: DefaultsAError,
    },
    #[error("resolving the provider defaults failed: {source}")]
    DefaultsB {
        #[source]
        source: DefaultsBError,
    },
}

pub struct ResolutionRequest<'context> {
    pub model: &'context str,
    pub custom_llm_provider: Option<&'context str>,
    pub api_base: Option<&'context str>,
    pub api_key: Option<&'context SecretString>,
}

// Every outside value the resolver reads is passed in here, so it never touches the process environment.
pub struct ResolutionContext<'context> {
    pub catalog: &'context ModelCatalog,
    pub known_model_names: &'context BTreeSet<String>,
    pub custom_providers: &'context BTreeSet<String>,
    pub environment: &'context BTreeMap<String, String>,
    pub registry: JSONProviderRegistry,
    pub policies: ProviderPolicies,
    pub globals: GlobalCredentials,
}

// Ports get_llm_provider lines 158-551: B209 classifies the declaration and this composes the defaults.
pub fn resolve(
    request: &ResolutionRequest<'_>,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedModel, ResolutionError> {
    let classification = classify_prefix(&PrefixClassificationInput {
        model: request.model,
        // An empty declaration names no provider, matching LiteLLM's falsy check on custom_llm_provider.
        custom_llm_provider: request.custom_llm_provider.filter(|name| !name.is_empty()),
        configured_model_names: context.known_model_names,
        catalog: context.catalog,
    });
    match classification {
        PrefixClassification::NonOpenaiAzure { model } => Ok(unchanged(
            model,
            ProviderIdentity::Known(ProviderId::Openai),
            request,
        )),
        PrefixClassification::OpenRouterNative { model }
        | PrefixClassification::OpenRouterNested { model } => Ok(unchanged(
            model,
            ProviderIdentity::Known(ProviderId::Openrouter),
            request,
        )),
        PrefixClassification::ExplicitProvider {
            provider,
            model,
            needs_provider_defaults,
        } if needs_provider_defaults => {
            defaults::resolve(ProviderIdentity::Known(provider), model, request, context)
        }
        PrefixClassification::ExplicitProvider {
            provider, model, ..
        } => Ok(unchanged(model, ProviderIdentity::Known(provider), request)),
        PrefixClassification::Unresolved {
            model,
            custom_llm_provider,
        } => unresolved(model, custom_llm_provider, request, context),
    }
}

// A branch that resolves no dynamic key keeps the model it was given and the caller's api_base.
fn unchanged(
    model: String,
    provider: ProviderIdentity,
    request: &ResolutionRequest<'_>,
) -> ResolvedModel {
    ResolvedModel {
        model,
        provider,
        api_base: request.api_base.map(str::to_owned),
        api_key: None,
    }
}

fn unresolved(
    model: String,
    custom_llm_provider: Option<String>,
    request: &ResolutionRequest<'_>,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedModel, ResolutionError> {
    if let Some((prefix, rest)) = model.split_once('/')
        && context.custom_providers.contains(prefix)
    {
        let identity = ProviderIdentity::Custom(prefix.to_owned());
        return Ok(unchanged(rest.to_owned(), identity, request));
    }
    if let Some(api_base) = request.api_base
        && let Some(matched) = endpoint::resolve(api_base, request, context)?
    {
        return Ok(ResolvedModel {
            model,
            provider: matched.provider,
            api_base: request.api_base.map(str::to_owned),
            api_key: matched.api_key,
        });
    }
    if let Some(inferred) = inference::infer(&model, context.catalog) {
        if inferred.needs_defaults {
            return defaults::resolve(inferred.identity, model, request, context);
        }
        return Ok(unchanged(model, inferred.identity, request));
    }
    // An unregistered declared provider string survives here in LiteLLM; betterlite keeps it as a custom name.
    if let Some(provider) = custom_llm_provider {
        let identity = ProviderIdentity::Custom(provider);
        return Ok(unchanged(model, identity, request));
    }
    Err(ResolutionError::MissingProvider { model })
}

mod tests;
