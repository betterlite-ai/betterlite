use betterlite_models::billing_text::{FormattedPromptData, PromptCallType, get_formatted_prompt};
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use serde::{Deserialize, Serialize};

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{
    EmbeddingInput, InputEntry, ResolvedFiles, flatten, has_nested, is_image_only, is_media,
    is_multimodal,
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
pub struct ModalityCount {
    pub modality: Modality,
    #[serde(rename = "tokenCount")]
    pub token_count: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    pub prompt_token_count: Option<i64>,
    #[serde(rename = "totalTokenCount")]
    pub total_token_count: Option<i64>,
    #[serde(rename = "promptTokensDetails")]
    pub prompt_tokens_details: Option<Vec<ModalityCount>>,
    #[serde(rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<i64>,
    #[serde(rename = "responseTokenCount")]
    pub response_token_count: Option<i64>,
    #[serde(rename = "cachedContentTokenCount")]
    pub cached_content_token_count: Option<i64>,
    #[serde(rename = "toolUsePromptTokenCount")]
    pub tool_use_prompt_token_count: Option<i64>,
    #[serde(rename = "thoughtsTokenCount")]
    pub thoughts_token_count: Option<i64>,
    #[serde(rename = "toolUsePromptTokensDetails")]
    pub tool_use_details: Option<Vec<ModalityCount>>,
    #[serde(rename = "cacheTokensDetails")]
    pub cache_tokens_details: Option<Vec<ModalityCount>>,
    #[serde(rename = "responseTokensDetails")]
    pub response_tokens_details: Option<Vec<ModalityCount>>,
    #[serde(rename = "candidatesTokensDetails")]
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
    let prompt_tokens = if is_multimodal(input) || has_nested(input) {
        let texts = text_elements(input);
        if texts.is_empty() {
            0
        } else {
            count_formatted(&Json::List(texts), model, counter)?
        }
    } else {
        count_input(input, model, counter)?
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
        return fallback_usage(input, model, counter);
    };
    let prompt_tokens = token_count(metadata.prompt_token_count.unwrap_or(0))?;
    let total_tokens = token_count(
        metadata
            .total_token_count
            .filter(|count| *count != 0)
            .unwrap_or_else(|| i64::from(prompt_tokens)),
    )?;
    let prompt_tokens_details = match metadata
        .prompt_tokens_details
        .filter(|details| !details.is_empty())
    {
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
        Some(details) => modality_tokens(&details)?,
    };
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens,
        prompt_tokens_details: Some(prompt_tokens_details),
    })
}

fn fallback_usage(
    input: &EmbeddingInput,
    model: &str,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingUsage, GeminiEmbeddingError> {
    if is_multimodal(input) {
        return Ok(EmbeddingUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        });
    }
    let prompt_tokens = count_input(input, model, counter)?;
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: None,
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

fn count_input(
    input: &EmbeddingInput,
    model: &str,
    counter: &dyn TextTokenCounter,
) -> Result<u32, GeminiEmbeddingError> {
    let value = match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Entries(entries) => Json::List(entries.iter().map(entry_value).collect()),
    };
    count_formatted(&value, model, counter)
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
