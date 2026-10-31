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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixClassification {
    NonOpenaiAzure {
        model: String,
    },
    ExplicitProvider {
        provider: ProviderId,
        model: String,
        needs_provider_defaults: bool,
    },
    OpenRouterNative {
        model: String,
    },
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

fn classify_normalized(
    normalized: NormalizedModel,
    configured_model_names: &BTreeSet<String>,
) -> PrefixClassification {
    let NormalizedModel {
        model,
        custom_llm_provider,
    } = normalized;

    match normalize_openrouter_prefix(&model, custom_llm_provider.as_deref()) {
        OpenRouterPrefix::Nested {
            model: nested_model,
        } => {
            return PrefixClassification::OpenRouterNested {
                model: nested_model,
            };
        }
        OpenRouterPrefix::Native {
            model: native_model,
        } => {
            return PrefixClassification::OpenRouterNative {
                model: native_model,
            };
        }
        OpenRouterPrefix::NotApplicable => {}
    }

    let Some((prefix, rest)) = model.split_once('/') else {
        return PrefixClassification::Unresolved {
            model,
            custom_llm_provider,
        };
    };
    let Some(provider) = ProviderId::parse(prefix) else {
        return PrefixClassification::Unresolved {
            model,
            custom_llm_provider,
        };
    };
    PrefixClassification::ExplicitProvider {
        provider,
        model: rest.to_owned(),
        needs_provider_defaults: !configured_model_names.contains(prefix),
    }
}

// Ports get_llm_provider lines 197-256; the provider registry and credential resolution it also runs are owned elsewhere.
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
