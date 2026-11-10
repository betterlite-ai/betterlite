use crate::catalog::ModelCatalog;
use crate::identity::anthropic_text::anthropic_text_provider;
use crate::identity::provider_id::ProviderId;
use crate::metadata::identity::Mode;

use super::ProviderIdentity;
use constants::{
    BASETEN_MODELS, BEDROCK_CONVERSE_MODELS, BEDROCK_EMBEDDING_MODELS, COHERE_EMBEDDING_MODELS,
    EMPOWER_MODELS, MARITALK_MODELS, OPEN_AI_EMBEDDING_MODELS, OPENAI_IMAGE_GENERATION_MODELS,
    OPENAI_VIDEO_GENERATION_MODELS, PETALS_MODELS, PREFIX_PROVIDERS, REPLICATE_ID_LENGTH,
    REPLICATE_MODELS,
};

mod constants;

pub(super) struct Inference {
    pub identity: ProviderIdentity,
    pub needs_defaults: bool,
}

impl From<ProviderId> for Inference {
    fn from(provider: ProviderId) -> Self {
        Self {
            identity: ProviderIdentity::Known(provider),
            needs_defaults: false,
        }
    }
}

// Ports the model-inference chain of get_llm_provider lines 411-531, in LiteLLM's order.
pub(super) fn infer(model: &str, catalog: &ModelCatalog) -> Option<Inference> {
    let entry = catalog.entries.get(model);
    let provider = entry.map(|found| found.fields.metadata.litellm_provider.as_str());
    let mode = entry.and_then(|found| found.fields.metadata.mode);
    openai_family(model, provider)
        .or_else(|| declared_family(model, provider))
        .or_else(|| cohere_family(model, provider))
        .or_else(|| vertex_family(model, provider))
        .or_else(|| named_family(model, provider, mode))
        .or_else(|| (model == "*").then_some(Inference::from(ProviderId::Openai)))
        .or_else(|| prefixed(model))
        .or_else(|| generalization(model, catalog))
}

// The finetune rule only keeps single-colon ft: keys out; a model named by the two ft: substrings resolves anyway.
fn openai_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    if provider == Some("openai") {
        let finetune = model.starts_with("ft:") && model.matches(':').count() <= 1;
        if !finetune {
            return Some(Inference::from(ProviderId::Openai));
        }
    }
    let fine_tuned = model.contains("ft:gpt-3.5-turbo") || model.contains("ft:gpt-4");
    let generated = OPENAI_IMAGE_GENERATION_MODELS.contains(&model)
        || model.starts_with("gpt-image")
        || OPENAI_VIDEO_GENERATION_MODELS.contains(&model);
    (fine_tuned || generated).then_some(Inference::from(ProviderId::Openai))
}

fn declared_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    match provider {
        Some("text-completion-openai") => Some(Inference::from(ProviderId::TextCompletionOpenai)),
        Some("anthropic") => {
            let (_, identified) = anthropic_text_provider(model, Some("anthropic"));
            let provider = if identified.as_deref() == Some("anthropic_text") {
                ProviderId::AnthropicText
            } else {
                ProviderId::Anthropic
            };
            Some(Inference::from(provider))
        }
        _ => None,
    }
}

fn cohere_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    match provider {
        Some("cohere") => return Some(Inference::from(ProviderId::Cohere)),
        Some("cohere_chat") => return Some(Inference::from(ProviderId::CohereChat)),
        _ => {}
    }
    if COHERE_EMBEDDING_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Cohere));
    }
    if is_replicate(model) {
        return Some(Inference::from(ProviderId::Replicate));
    }
    if provider == Some("openrouter") {
        return Some(Inference::from(ProviderId::Openrouter));
    }
    MARITALK_MODELS
        .contains(&model)
        .then_some(Inference::from(ProviderId::Maritalk))
}

fn vertex_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    let name = provider?;
    if name.starts_with("vertex_ai-")
        && (is_vertex_text_set(name) || vertex_name_matches(model, name))
    {
        return Some(Inference::from(ProviderId::VertexAi));
    }
    None
}

fn is_vertex_text_set(name: &str) -> bool {
    matches!(
        name,
        "vertex_ai-chat-models"
            | "vertex_ai-code-chat-models"
            | "vertex_ai-text-models"
            | "vertex_ai-code-text-models"
            | "vertex_ai-language-models"
            | "vertex_ai-embedding-models"
            | "vertex_ai-vision-models"
    )
}

// Image and video keys enter their sets with the "vertex_ai/" namespace stripped away.
fn vertex_name_matches(model: &str, name: &str) -> bool {
    matches!(name, "vertex_ai-image-models" | "vertex_ai-video-models")
        && !model.contains("vertex_ai/")
}

fn named_family(model: &str, provider: Option<&str>, mode: Option<Mode>) -> Option<Inference> {
    if provider == Some("ai21") {
        return Some(Inference {
            identity: ProviderIdentity::Known(ProviderId::Ai21Chat),
            needs_defaults: true,
        });
    }
    aleph_baseten_nlp_petals(model, provider)
        .or_else(|| {
            is_bedrock(model, provider, mode).then_some(Inference::from(ProviderId::Bedrock))
        })
        .or_else(|| remaining_named_providers(model, provider))
}

fn aleph_baseten_nlp_petals(model: &str, provider: Option<&str>) -> Option<Inference> {
    match provider {
        // aleph_alpha is a provider slug LiteLLM knows but its typed provider list does not.
        Some("aleph_alpha") => {
            return Some(Inference {
                identity: ProviderIdentity::Custom(String::from("aleph_alpha")),
                needs_defaults: false,
            });
        }
        Some("nlp_cloud") => return Some(Inference::from(ProviderId::NlpCloud)),
        _ => {}
    }
    if BASETEN_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Baseten));
    }
    PETALS_MODELS
        .contains(&model)
        .then_some(Inference::from(ProviderId::Petals))
}

fn remaining_named_providers(model: &str, provider: Option<&str>) -> Option<Inference> {
    match provider {
        Some("watsonx") => return Some(Inference::from(ProviderId::Watsonx)),
        _ if OPEN_AI_EMBEDDING_MODELS.contains(&model) => {
            return Some(Inference::from(ProviderId::Openai));
        }
        _ => {}
    }
    if provider == Some("empower") || EMPOWER_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Empower));
    }
    (provider == Some("gradient_ai")).then_some(Inference::from(ProviderId::GradientAi))
}

fn is_bedrock(model: &str, provider: Option<&str>, mode: Option<Mode>) -> bool {
    if provider == Some("bedrock_converse")
        || BEDROCK_EMBEDDING_MODELS.contains(&model)
        || BEDROCK_CONVERSE_MODELS.contains(&model)
    {
        return true;
    }
    provider == Some("bedrock")
        && mode != Some(Mode::Guardrail)
        && !is_bedrock_pricing_only_model(model)
}

fn prefixed(model: &str) -> Option<Inference> {
    PREFIX_PROVIDERS.iter().find_map(|(prefix, provider)| {
        model
            .starts_with(prefix)
            .then_some(Inference::from(*provider))
    })
}

fn generalization(model: &str, catalog: &ModelCatalog) -> Option<Inference> {
    let slug = catalog.generalizations.match_routing(model)?;
    let identity = slug.parse::<ProviderId>().map_or_else(
        |_| ProviderIdentity::Custom(slug.to_owned()),
        ProviderIdentity::Known,
    );
    Some(Inference {
        identity,
        needs_defaults: false,
    })
}

fn is_replicate(model: &str) -> bool {
    if REPLICATE_MODELS.contains(&model) {
        return true;
    }
    let mut parts = model.split(':');
    let _name = parts.next();
    parts
        .next()
        .is_some_and(|id| id.chars().count() == REPLICATE_ID_LENGTH)
}

// is_bedrock_pricing_only_model: bedrock/<region>/<model> keys and month-commitment keys price only.
fn is_bedrock_pricing_only_model(model: &str) -> bool {
    if model.contains("month-commitment") {
        return true;
    }
    let Some(rest) = model.strip_prefix("bedrock/") else {
        return false;
    };
    let Some((region, name)) = rest.split_once('/') else {
        return false;
    };
    !region.is_empty()
        && !name.is_empty()
        && region
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}
