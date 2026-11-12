use betterlite_models::billing_text::{FormattedPromptData, PromptCallType, get_formatted_prompt};
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use serde::{Deserialize, Serialize};

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{
    EmbeddingInput, InputEntry, ResolvedFiles, flatten, is_image_only, is_media, shape,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModalityTokens {
    #[serde(rename = "text_tokens")]
    pub text: Option<u32>,
    #[serde(rename = "audio_tokens")]
    pub audio: Option<u32>,
    #[serde(rename = "image_tokens")]
    pub image: Option<u32>,
    #[serde(rename = "video_tokens")]
    pub video: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_tokens_details: Option<ModalityTokens>,
}
pub trait TextTokenCounter {
    fn count_text(&self, model: &str, text: &str) -> Result<u32, CountError>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Modality {
    Text,
    Audio,
    Image,
    Video,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModalityCount {
    pub modality: Modality,
    pub token_count: i64,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: Option<i64>,
    pub total_token_count: Option<i64>,
    pub prompt_tokens_details: Option<Vec<ModalityCount>>,
    pub candidates_token_count: Option<i64>,
    pub response_token_count: Option<i64>,
    pub cached_content_token_count: Option<i64>,
    pub tool_use_prompt_token_count: Option<i64>,
    pub thoughts_token_count: Option<i64>,
    #[serde(rename = "toolUsePromptTokensDetails")]
    pub tool_use_details: Option<Vec<ModalityCount>>,
    pub cache_tokens_details: Option<Vec<ModalityCount>>,
    pub response_tokens_details: Option<Vec<ModalityCount>>,
    pub candidates_tokens_details: Option<Vec<ModalityCount>>,
}
#[must_use]
pub fn parse_usage_metadata(raw: Option<&Json>) -> Option<UsageMetadata> {
    let Json::Object(_) = raw? else {
        return None;
    };
    serde_json::from_value(serde_json::to_value(raw).ok()?).ok()
}
pub fn batch_usage(
    input: &EmbeddingInput,
    model: &str,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingUsage, GeminiEmbeddingError> {
    let shape = shape(input);
    let prompt_tokens = if shape.media || shape.nested {
        let texts = text_elements(input);
        if texts.is_empty() {
            0
        } else {
            count_formatted(&Json::List(texts), model, counter)?
        }
    } else {
        count_formatted(&input_value(input), model, counter)?
    };
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: None,
    })
}
pub fn embed_content_usage(
    input: &EmbeddingInput,
    model: &str,
    raw_usage_metadata: Option<&Json>,
    resolved_files: &ResolvedFiles,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingUsage, GeminiEmbeddingError> {
    let Some(metadata) = parse_usage_metadata(raw_usage_metadata) else {
        let prompt_tokens = fallback_tokens(input, model, counter)?;
        return Ok(EmbeddingUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
            prompt_tokens_details: None,
        });
    };
    let prompt_tokens = token_count(metadata.prompt_token_count.unwrap_or(0))?;
    let total_tokens = token_count(
        metadata
            .total_token_count
            .filter(|count| *count != 0)
            .unwrap_or_else(|| i64::from(prompt_tokens)),
    )?;
    let details = metadata
        .prompt_tokens_details
        .filter(|details| !details.is_empty());
    let prompt_tokens_details = match details {
        None => ModalityTokens {
            text: Some(0),
            audio: None,
            image: Some(if is_image_only(input, resolved_files) {
                prompt_tokens
            } else {
                0
            }),
            video: None,
        },
        Some(counts) => modality_tokens(&counts)?,
    };
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens,
        prompt_tokens_details: Some(prompt_tokens_details),
    })
}
fn modality_tokens(details: &[ModalityCount]) -> Result<ModalityTokens, GeminiEmbeddingError> {
    let mut text = 0_i64;
    let mut audio = 0_i64;
    let mut image = 0_i64;
    let mut video = 0_i64;
    for detail in details {
        let total = match detail.modality {
            Modality::Text => &mut text,
            Modality::Audio => &mut audio,
            Modality::Image => &mut image,
            Modality::Video => &mut video,
        };
        *total = total
            .checked_add(detail.token_count)
            .ok_or(GeminiEmbeddingError::TokenOverflow)?;
    }
    Ok(ModalityTokens {
        text: Some(token_count(text)?),
        audio: Some(token_count(audio)?),
        image: Some(token_count(image)?),
        video: Some(token_count(video)?),
    })
}
fn token_count(value: i64) -> Result<u32, GeminiEmbeddingError> {
    match u32::try_from(value) {
        Ok(count) => Ok(count),
        Err(_error) => Err(GeminiEmbeddingError::TokenRange { value }),
    }
}
fn fallback_tokens(
    input: &EmbeddingInput,
    model: &str,
    counter: &dyn TextTokenCounter,
) -> Result<u32, GeminiEmbeddingError> {
    if shape(input).media {
        return Ok(0);
    }
    count_formatted(&input_value(input), model, counter)
}
fn input_value(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Entries(entries) => Json::List(entries.iter().map(entry_value).collect()),
    }
}
fn entry_value(entry: &InputEntry) -> Json {
    match entry {
        InputEntry::Text(text) => Json::Text(text.clone()),
        InputEntry::Group(group) => {
            Json::List(group.iter().map(|text| Json::Text(text.clone())).collect())
        }
    }
}
fn count_formatted(
    value: &Json,
    model: &str,
    counter: &dyn TextTokenCounter,
) -> Result<u32, GeminiEmbeddingError> {
    let data = FormattedPromptData {
        messages: None,
        prompt: None,
        input: Some(value),
    };
    let text = get_formatted_prompt(&data, PromptCallType::Embedding)
        .map_err(|source| GeminiEmbeddingError::Formatting { source })?;
    counter
        .count_text(model, &text)
        .map_err(|source| GeminiEmbeddingError::Counting { source })
}
fn text_elements(input: &EmbeddingInput) -> Vec<Json> {
    let mut texts = Vec::new();
    for element in flatten(input) {
        if !is_media(element) {
            texts.push(Json::Text(element.to_owned()));
        }
    }
    texts
}
