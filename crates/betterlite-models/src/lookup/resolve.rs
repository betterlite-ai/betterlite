use std::collections::BTreeSet;

use serde::Deserialize;

use crate::catalog::{CatalogEntry, ModelCatalog};
use crate::catalog_generalizations::ModelInfo;
use crate::identity::explicit_prefix::{
    PrefixClassification, PrefixClassificationInput, classify_prefix,
};
use crate::metadata::identity::Mode;
use crate::model_catalog_keys::{EntryProvider, check_provider_match};

use super::capabilities::CapabilityView;
use super::info::{ModelInfoError, ModelQuery};
use super::limits::ModelLimits;
use super::names::potential_names;
use super::provider_info::provider_info;
use super::remap::remap;

// B2203's bedrock normalizers live in betterlite-context, which depends on this crate, so the caller passes them in.
#[derive(Clone, Copy, Debug)]
pub struct BedrockNames {
    pub base_model: fn(&str) -> String,
    pub strip_routing_prefix: fn(&str) -> String,
    pub strip_mantle_region: fn(&str) -> String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitModel {
    pub provider: Option<String>,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub key: String,
    pub provider: Option<String>,
    pub mode: Option<Mode>,
    pub limits: ModelLimits,
    pub capabilities: CapabilityView,
}

struct Found<'catalog> {
    key: String,
    entry: &'catalog CatalogEntry,
}

#[derive(Deserialize)]
struct Identity {
    litellm_provider: Option<String>,
    mode: Option<Mode>,
}

struct Decoded {
    identity: Identity,
    limits: ModelLimits,
    capabilities: CapabilityView,
}

fn decode(info: &ModelInfo) -> Result<Decoded, ModelInfoError> {
    // each struct reads its own keys from the same text, since flatten would round the numbers
    let json = serde_json::to_string(info)?;
    Ok(Decoded {
        identity: serde_json::from_str(&json)?,
        limits: serde_json::from_str(&json)?,
        capabilities: serde_json::from_str(&json)?,
    })
}

// Stands in for get_llm_provider: the explicit prefix, the exact entry's provider, then the routing rules.
#[must_use]
pub fn split_model(catalog: &ModelCatalog, requested: &str) -> SplitModel {
    let classified = classify_prefix(&PrefixClassificationInput {
        model: requested,
        custom_llm_provider: None,
        configured_model_names: &BTreeSet::new(),
        catalog,
    });
    match classified {
        PrefixClassification::ExplicitProvider {
            provider, model, ..
        } => SplitModel {
            provider: Some(provider.to_string()),
            model,
        },
        PrefixClassification::NonOpenaiAzure { model } => SplitModel {
            provider: Some("openai".to_owned()),
            model,
        },
        PrefixClassification::OpenRouterNative { model }
        | PrefixClassification::OpenRouterNested { model } => SplitModel {
            provider: Some("openrouter".to_owned()),
            model,
        },
        PrefixClassification::Unresolved { model, .. } => {
            let declared = catalog
                .lookup
                .get(model.as_str())
                .and_then(|canonical| catalog.entries.get(canonical.as_ref()))
                .map(|entry| entry.fields.metadata.litellm_provider.as_str())
                .filter(|provider| !provider.is_empty())
                .or_else(|| catalog.generalizations.match_routing(&model));
            SplitModel {
                provider: declared.map(str::to_owned),
                model,
            }
        }
    }
}

fn exact_entry<'catalog>(
    catalog: &'catalog ModelCatalog,
    candidate: &str,
) -> Option<Found<'catalog>> {
    let (key, canonical) = catalog.lookup.get_key_value(candidate).or_else(|| {
        let lowercase = candidate.to_lowercase();
        catalog
            .lookup
            .iter()
            .find(|(key, _)| key.to_lowercase() == lowercase)
    })?;
    Some(Found {
        key: key.clone(),
        entry: catalog.entries.get(canonical.as_ref())?,
    })
}

fn compatible(entry: &CatalogEntry, provider: Option<&str>) -> bool {
    let declared = entry.fields.metadata.litellm_provider.as_str();
    let entry_provider = if declared.is_empty() {
        EntryProvider::Unconstrained
    } else {
        EntryProvider::Text(declared)
    };
    matches!(check_provider_match(entry_provider, provider), Ok(true))
}

// Chat and responses entries take the fallback rules' values for the fields they leave absent.
fn from_entry(
    catalog: &ModelCatalog,
    found: &Found<'_>,
    provider: Option<&str>,
) -> Result<Resolved, ModelInfoError> {
    let metadata = &found.entry.fields.metadata;
    let mut resolved = Resolved {
        key: found.key.clone(),
        provider: Some(metadata.litellm_provider.clone())
            .filter(|declared| !declared.is_empty())
            .or_else(|| provider.map(str::to_owned)),
        mode: metadata.mode,
        limits: ModelLimits::from_fields(metadata),
        capabilities: CapabilityView::from_fields(&found.entry.fields.capabilities),
    };
    if matches!(metadata.mode, None | Some(Mode::Chat | Mode::Responses))
        && let Some(fill) = catalog
            .generalizations
            .match_fill_missing(&found.key, &metadata.litellm_provider)
    {
        let filled = decode(&fill)?;
        resolved.mode = resolved.mode.or(filled.identity.mode);
        resolved.limits = resolved.limits.or(filled.limits);
        resolved.capabilities = resolved.capabilities.or(filled.capabilities);
    }
    Ok(resolved)
}

// Rules lose to exact entries: a candidate that is a catalog key makes the model known but provider-mismatched.
fn generalized(
    catalog: &ModelCatalog,
    candidates: &[&str],
    provider: Option<&str>,
) -> Result<Option<Resolved>, ModelInfoError> {
    if candidates
        .iter()
        .any(|candidate| exact_entry(catalog, candidate).is_some())
    {
        return Ok(None);
    }
    for candidate in candidates {
        let Some(info) = catalog.generalizations.match_capabilities(candidate) else {
            continue;
        };
        let decoded = decode(&info)?;
        return Ok(Some(Resolved {
            key: (*candidate).to_owned(),
            provider: provider
                .map(str::to_owned)
                .or(decoded.identity.litellm_provider),
            mode: decoded.identity.mode,
            limits: decoded.limits,
            capabilities: decoded.capabilities,
        }));
    }
    Ok(None)
}

// LiteLLM fetches max_position_embeddings from huggingface.co for these; betterlite sends no request built from a model name.
fn huggingface(model: &str) -> Resolved {
    Resolved {
        key: model.to_owned(),
        provider: Some("huggingface".to_owned()),
        mode: Some(Mode::Chat),
        limits: ModelLimits::default(),
        capabilities: CapabilityView::default(),
    }
}

fn lookup_candidates(
    catalog: &ModelCatalog,
    bedrock: &BedrockNames,
    query: &ModelQuery<'_>,
) -> Result<Option<Resolved>, ModelInfoError> {
    let (model, remapped_provider) = remap(catalog, query.model, query.provider);
    let remapped = ModelQuery {
        model: &model,
        provider: remapped_provider,
    };
    let names = potential_names(catalog, bedrock, &remapped);
    let provider = names.provider.as_deref();
    if provider == Some("huggingface") {
        return Ok(Some(huggingface(&model)));
    }
    let candidates = [
        names.combined.as_str(),
        model.as_str(),
        names.split_model.as_str(),
        names.combined_stripped.as_str(),
        names.stripped.as_str(),
        names.provider_prefixed.as_str(),
    ];
    for candidate in candidates {
        if let Some(found) = exact_entry(catalog, candidate)
            && compatible(found.entry, provider)
        {
            return from_entry(catalog, &found, provider).map(Some);
        }
    }
    generalized(catalog, &candidates, provider)
}

// The caller's provider and model pick the provider overlay; the remapped ones pick the entry.
pub fn find_info(
    catalog: &ModelCatalog,
    bedrock: &BedrockNames,
    query: &ModelQuery<'_>,
) -> Result<Option<Resolved>, ModelInfoError> {
    let Some(mut resolved) = lookup_candidates(catalog, bedrock, query)? else {
        return Ok(None);
    };
    let overlay = provider_info(catalog, query.model, query.provider);
    resolved.capabilities = overlay.or(resolved.capabilities);
    Ok(Some(resolved))
}
