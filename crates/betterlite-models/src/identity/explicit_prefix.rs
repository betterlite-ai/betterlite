use std::collections::BTreeSet;

use crate::catalog::ModelCatalog;
use crate::identity::anthropic_text::anthropic_text_provider;
use crate::identity::azure::is_non_openai_azure_model;
use crate::identity::cohere::cohere_chat_provider;
use crate::identity::openrouter::{OpenRouterPrefix, normalize_openrouter_prefix};
use crate::identity::provider_id::ProviderId;

pub struct PrefixClassificationInput<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub configured_model_names: &'input BTreeSet<String>,
    pub catalog: &'input ModelCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrefixClassification {
    NonOpenaiAzure {
        model: String,
    },
    ExplicitProvider {
        provider: ProviderId,
        model: String,
        needs_provider_defaults: bool,
    },
    #[serde(rename = "openrouter_native")]
    OpenRouterNative {
        model: String,
    },
    #[serde(rename = "openrouter_nested")]
    OpenRouterNested {
        model: String,
    },
    Unresolved {
        model: String,
        custom_llm_provider: Option<String>,
    },
}

struct NormalizedModel {
    model: String,
    custom_llm_provider: Option<String>,
}

// A JSONProviderRegistry hit always needs provider defaults, ignoring model_list_set (lines 223-232).
const JSON_REGISTERED_PROVIDERS: [ProviderId; 31] = [
    ProviderId::Publicai,
    ProviderId::Helicone,
    ProviderId::Veniceai,
    ProviderId::XiaomiMimo,
    ProviderId::Scaleway,
    ProviderId::Synthetic,
    ProviderId::Apertis,
    ProviderId::Nanogpt,
    ProviderId::Poe,
    ProviderId::Chutes,
    ProviderId::Abliteration,
    ProviderId::Llamagate,
    ProviderId::Gmi,
    ProviderId::Sarvam,
    ProviderId::Assemblyai,
    ProviderId::CharityEngine,
    ProviderId::Aihubmix,
    ProviderId::Crusoe,
    ProviderId::Darkbloom,
    ProviderId::Neosantara,
    ProviderId::Tensormesh,
    ProviderId::Parasail,
    ProviderId::Libertai,
    ProviderId::Empiriolabs,
    ProviderId::Meta,
    ProviderId::Cognition,
    ProviderId::Cortecs,
    ProviderId::Pinstripes,
    ProviderId::ScxAi,
    ProviderId::Prism,
    ProviderId::Sail,
];

fn normalize_declared_provider(
    model: &str,
    custom_llm_provider: Option<&str>,
    catalog: &ModelCatalog,
) -> NormalizedModel {
    let (cohere_model, cohere_provider) = cohere_chat_provider(model, custom_llm_provider, catalog);
    let (anthropic_model, anthropic_provider) =
        anthropic_text_provider(&cohere_model, cohere_provider.as_deref());
    let prefixed_model = match &anthropic_provider {
        Some(provider)
            if anthropic_model
                .split('/')
                .next()
                .unwrap_or(&anthropic_model)
                != provider.as_str() =>
        {
            format!("{provider}/{anthropic_model}")
        }
        _ => anthropic_model,
    };
    NormalizedModel {
        model: prefixed_model,
        custom_llm_provider: anthropic_provider,
    }
}

fn classify_openrouter(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Option<PrefixClassification> {
    match normalize_openrouter_prefix(model, custom_llm_provider) {
        OpenRouterPrefix::Nested {
            model: nested_model,
        } => Some(PrefixClassification::OpenRouterNested {
            model: nested_model,
        }),
        OpenRouterPrefix::Native {
            model: native_model,
        } => Some(PrefixClassification::OpenRouterNative {
            model: native_model,
        }),
        OpenRouterPrefix::NotApplicable => None,
    }
}

fn classify_explicit_provider(
    provider: ProviderId,
    prefix: &str,
    rest: &str,
    configured_model_names: &BTreeSet<String>,
) -> PrefixClassification {
    let needs_provider_defaults =
        JSON_REGISTERED_PROVIDERS.contains(&provider) || !configured_model_names.contains(prefix);
    PrefixClassification::ExplicitProvider {
        provider,
        model: rest.to_owned(),
        needs_provider_defaults,
    }
}

fn classify_normalized(
    normalized: NormalizedModel,
    configured_model_names: &BTreeSet<String>,
) -> PrefixClassification {
    let NormalizedModel {
        model,
        custom_llm_provider,
    } = normalized;

    if let Some(classification) = classify_openrouter(&model, custom_llm_provider.as_deref()) {
        return classification;
    }

    let Some((prefix, rest)) = model.split_once('/') else {
        return PrefixClassification::Unresolved {
            model,
            custom_llm_provider,
        };
    };
    let Ok(provider) = prefix.parse::<ProviderId>() else {
        return PrefixClassification::Unresolved {
            model,
            custom_llm_provider,
        };
    };
    classify_explicit_provider(provider, prefix, rest, configured_model_names)
}

// Ports get_llm_provider lines 197-256; the registry decoder and credential resolution are owned elsewhere.
#[must_use]
pub fn classify_prefix(input: &PrefixClassificationInput<'_>) -> PrefixClassification {
    let PrefixClassificationInput {
        model,
        custom_llm_provider,
        configured_model_names,
        catalog,
    } = *input;

    if model.split('/').next().unwrap_or(model) == "azure"
        && is_non_openai_azure_model(model, catalog)
    {
        return PrefixClassification::NonOpenaiAzure {
            model: model.to_owned(),
        };
    }

    let normalized = normalize_declared_provider(model, custom_llm_provider, catalog);
    classify_normalized(normalized, configured_model_names)
}

mod tests;
