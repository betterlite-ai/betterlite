use std::collections::BTreeSet;

use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoresConfigTag, select_vector_stores_config,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::explicit_prefix::{
    PrefixClassification, PrefixClassificationInput, classify_prefix,
};
use betterlite_models::identity::provider_id::ProviderId;

use super::error::VectorStoreSearchError;

pub struct ResolvedProvider {
    pub provider: ProviderId,
    pub api_type: Option<String>,
    pub tag: VectorStoresConfigTag,
}

// LiteLLM's provider default, get_llm_provider for a slash and LlmProviders(...) for a bare name, then B010's config
pub fn resolve_provider(
    operation: &'static str,
    custom_llm_provider: Option<&str>,
) -> Result<ResolvedProvider, VectorStoreSearchError> {
    let requested = custom_llm_provider.unwrap_or("openai");
    let (provider, api_type) = if requested.contains('/') {
        prefixed(requested)?
    } else {
        let parsed = requested.parse::<ProviderId>().map_err(|_unparsed| {
            VectorStoreSearchError::InvalidProvider {
                name: requested.to_owned(),
            }
        })?;
        (parsed, None)
    };
    let tag = select_vector_stores_config(provider, api_type.as_deref()).ok_or_else(|| {
        VectorStoreSearchError::Unsupported {
            operation,
            provider: provider.to_string(),
        }
    })?;
    Ok(ResolvedProvider {
        provider,
        api_type,
        tag,
    })
}

// B174 over the whole string: a known prefix is the provider and the rest is the api_type
fn prefixed(model: &str) -> Result<(ProviderId, Option<String>), VectorStoreSearchError> {
    let configured = BTreeSet::new();
    let catalog = ModelCatalog::default();
    let input = PrefixClassificationInput {
        model,
        custom_llm_provider: None,
        configured_model_names: &configured,
        catalog: &catalog,
    };
    match classify_prefix(&input) {
        PrefixClassification::ExplicitProvider {
            provider,
            model: rest,
            ..
        } => Ok((provider, Some(rest))),
        PrefixClassification::NonOpenaiAzure { model: rest } => {
            Ok((ProviderId::Openai, Some(rest)))
        }
        PrefixClassification::OpenRouterNative { model: rest }
        | PrefixClassification::OpenRouterNested { model: rest } => {
            Ok((ProviderId::Openrouter, Some(rest)))
        }
        PrefixClassification::Unresolved { .. } => {
            Err(VectorStoreSearchError::UnresolvedProvider {
                model: model.to_owned(),
            })
        }
    }
}
