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
        .or_else(|| later_family(model, provider, mode))
        .or_else(|| (model == "*").then_some(Inference::from(ProviderId::Openai)))
        .or_else(|| prefixed(model))
        .or_else(|| {
            let slug = catalog.generalizations.match_routing(model)?;
            let identity = slug.parse::<ProviderId>().map_or_else(
                |_| ProviderIdentity::Custom(slug.to_owned()),
                ProviderIdentity::Known,
            );
            Some(Inference {
                identity,
                needs_defaults: false,
            })
        })
}

// The finetune rule keeps only single-colon ft: keys out of the openai set; the ft: substrings win anyway.
fn openai_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    let finetune = model.starts_with("ft:") && model.matches(':').count() <= 1;
    if provider == Some("openai") && !finetune {
        return Some(Inference::from(ProviderId::Openai));
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
            let text = identified.as_deref() == Some("anthropic_text");
            let identity = if text {
                ProviderIdentity::Known(ProviderId::AnthropicText)
            } else {
                ProviderIdentity::Known(ProviderId::Anthropic)
            };
            Some(Inference {
                identity,
                needs_defaults: false,
            })
        }
        _ => named_by_model(model, provider),
    }
}

fn named_by_model(model: &str, provider: Option<&str>) -> Option<Inference> {
    if provider == Some("cohere") || COHERE_EMBEDDING_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Cohere));
    }
    if provider == Some("cohere_chat") {
        return Some(Inference::from(ProviderId::CohereChat));
    }
    if is_replicate(model) {
        return Some(Inference::from(ProviderId::Replicate));
    }
    if provider == Some("openrouter") {
        return Some(Inference::from(ProviderId::Openrouter));
    }
    if MARITALK_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Maritalk));
    }
    vertex_family(model, provider)
}

fn vertex_family(model: &str, provider: Option<&str>) -> Option<Inference> {
    let name = provider?;
    let listed = is_vertex_text_set(name) || vertex_name_matches(model, name);
    (name.starts_with("vertex_ai-") && listed).then_some(Inference::from(ProviderId::VertexAi))
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

fn later_family(model: &str, provider: Option<&str>, mode: Option<Mode>) -> Option<Inference> {
    if provider == Some("ai21") {
        return Some(Inference {
            identity: ProviderIdentity::Known(ProviderId::Ai21Chat),
            needs_defaults: true,
        });
    }
    // aleph_alpha is a provider slug LiteLLM knows but its typed provider list does not.
    if provider == Some("aleph_alpha") {
        return Some(Inference {
            identity: ProviderIdentity::Custom(String::from("aleph_alpha")),
            needs_defaults: false,
        });
    }
    if BASETEN_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Baseten));
    }
    if provider == Some("nlp_cloud") {
        return Some(Inference::from(ProviderId::NlpCloud));
    }
    if PETALS_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Petals));
    }
    if is_bedrock(model, provider, mode) {
        return Some(Inference::from(ProviderId::Bedrock));
    }
    if provider == Some("watsonx") {
        return Some(Inference::from(ProviderId::Watsonx));
    }
    if OPEN_AI_EMBEDDING_MODELS.contains(&model) {
        return Some(Inference::from(ProviderId::Openai));
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
    // A bedrock/<region>/<model> key never reaches here: the bedrock/ prefix classifies first.
    provider == Some("bedrock")
        && mode != Some(Mode::Guardrail)
        && !model.contains("month-commitment")
}

fn prefixed(model: &str) -> Option<Inference> {
    PREFIX_PROVIDERS.iter().find_map(|(prefix, provider)| {
        model
            .starts_with(prefix)
            .then_some(Inference::from(*provider))
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
