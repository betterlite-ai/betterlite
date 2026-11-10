use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, Usage};

use super::StreamedChunks;
use super::records::{Choice, Chunk};

// ModelResponse.__init__ overwrites whatever object the first chunk passed, so the response always says this.
const RESPONSE_OBJECT: &str = "chat.completion";
const DEFAULT_FINISH_REASON: &str = "stop";
const DEFAULT_ROLE: &str = "assistant";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Vec<BaseChoice>,
    pub usage: Usage,
    pub hidden_params: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseChoice {
    pub index: i64,
    pub message: BaseMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseMessage {
    pub role: String,
    pub content: Option<String>,
}

#[must_use]
pub fn build_base_response(chunks: &StreamedChunks) -> BaseResponse {
    let first = &chunks.first_chunk;
    let first_model = first.model.as_deref();
    let mut response = BaseResponse {
        id: chunk_id(&chunks.chunks),
        object: RESPONSE_OBJECT.to_owned(),
        created: first.created.unwrap_or(0),
        model: model_from_chunks(&chunks.chunks, first_model),
        system_fingerprint: first.system_fingerprint.clone(),
        service_tier: service_tier_from_chunks(&chunks.chunks),
        choices: vec![BaseChoice {
            index: 0,
            message: BaseMessage {
                role: role_from_chunks(&chunks.chunks),
                content: Some(String::new()),
            },
            finish_reason: finish_reason(&chunks.chunks),
        }],
        usage: Usage::default(),
        hidden_params: JsonObject::new(),
    };
    apply_hidden_params(&mut response, chunks.chunks.last());
    if let Some(model) = provider_response_model(&chunks.chunks, first_model) {
        response.hidden_params.insert(
            "provider_response_model".to_owned(),
            JsonValue::String(model),
        );
    }
    response
}

pub fn apply_hidden_params(response: &mut BaseResponse, chunk: Option<&Chunk>) {
    if let Some(value) = chunk {
        response.hidden_params = value.hidden_object().cloned().unwrap_or_default();
    }
}

fn chunk_id(chunks: &[Chunk]) -> String {
    for chunk in chunks {
        if let Some(id) = chunk.id.as_deref().filter(|id| !id.is_empty()) {
            return id.to_owned();
        }
    }
    String::new()
}

fn role_from_chunks(chunks: &[Chunk]) -> String {
    chunks
        .iter()
        .find_map(|chunk| chunk.choices.first())
        .map_or_else(|| DEFAULT_ROLE.to_owned(), role_of_choice)
}

// Only the first choice of the first chunk that has choices is read; later roles are ignored.
fn role_of_choice(choice: &Choice) -> String {
    match choice
        .delta
        .as_ref()
        .and_then(|delta| delta.role.as_deref())
    {
        Some(role) if !role.is_empty() => role.to_owned(),
        Some(_) | None => DEFAULT_ROLE.to_owned(),
    }
}

fn model_from_chunks(chunks: &[Chunk], first_model: Option<&str>) -> Option<String> {
    let models = models_in_order(chunks, ModelSource::Chunk);
    first_differing(&models, first_model)
        .or(first_model)
        .map(str::to_owned)
}

fn service_tier_from_chunks(chunks: &[Chunk]) -> Option<String> {
    chunks.iter().rev().find_map(|chunk| {
        chunk
            .service_tier
            .as_deref()
            .filter(|tier| !tier.is_empty())
            .map(str::to_owned)
    })
}

fn finish_reason(chunks: &[Chunk]) -> String {
    let mut finish_reason = DEFAULT_FINISH_REASON.to_owned();
    for chunk in chunks {
        if let Some(choice) = chunk.choices.first()
            && let Some(reason) = &choice.finish_reason
        {
            finish_reason.clone_from(reason);
        }
    }
    finish_reason
}

fn provider_response_model(chunks: &[Chunk], first_model: Option<&str>) -> Option<String> {
    let models = models_in_order(chunks, ModelSource::ProviderResponse);
    first_differing(&models, first_model)
        .or_else(|| models.first().copied())
        .map(str::to_owned)
}

#[derive(Clone, Copy)]
enum ModelSource {
    Chunk,
    ProviderResponse,
}

fn models_in_order(chunks: &[Chunk], source: ModelSource) -> Vec<&str> {
    chunks
        .iter()
        .filter_map(|chunk| match source {
            ModelSource::Chunk => chunk.model.as_deref().filter(|model| !model.is_empty()),
            ModelSource::ProviderResponse => chunk
                .hidden_object()?
                .get("provider_response_model")?
                .as_str()
                .filter(|model| !model.is_empty()),
        })
        .collect()
}

fn first_differing<'model>(
    models: &[&'model str],
    first_model: Option<&str>,
) -> Option<&'model str> {
    models
        .iter()
        .copied()
        .find(|model| Some(*model) != first_model)
}
