use betterlite_models::billing_text::{FormattedPromptData, PromptCallType, get_formatted_prompt};
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use serde::Serialize;

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{
    EmbeddingInput, InputEntry, ResolvedFiles, flatten, has_nested, is_image_only, is_multimodal,
    is_multimodal_element,
};
use crate::gemini_embedding_records::usage_metadata::{
    Modality, ModalityCount, parse_usage_metadata,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModalityTokens {
    pub text_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
    pub image_tokens: Option<u32>,
    pub video_tokens: Option<u32>,
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
            .unwrap_or(i64::from(prompt_tokens)),
    )?;
    let prompt_tokens_details = match metadata
        .prompt_tokens_details
        .filter(|details| !details.is_empty())
    {
        None => ModalityTokens {
            text_tokens: Some(0),
            audio_tokens: None,
            image_tokens: Some(if is_image_only(input, resolved_files) {
                prompt_tokens
            } else {
                0
            }),
            video_tokens: None,
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
        text_tokens: Some(token_count(text)?),
        audio_tokens: Some(token_count(audio)?),
        image_tokens: Some(token_count(image)?),
        video_tokens: Some(token_count(video)?),
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
        if !is_multimodal_element(element) {
            texts.push(Json::Text(element.to_owned()));
        }
    }
    texts
}
