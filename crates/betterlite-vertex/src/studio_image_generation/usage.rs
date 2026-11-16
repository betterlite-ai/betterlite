use betterlite_models::decisions::{Extra, Json as ResponseJson};
use betterlite_openai::api::image_contracts::records::ImageInputTokens;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_with::{DefaultOnNull, serde_as};

use super::error::StudioImageError;

const PROMPT_DETAILS: &str = "promptTokensDetails";
const CANDIDATE_DETAILS: &str = "candidatesTokensDetails";

// the usageMetadata fields the Studio adapter reads, with absent and null counts both taking zero
#[serde_as]
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageUsageMetadata {
    #[serde_as(as = "DefaultOnNull")]
    pub prompt_token_count: i64,
    #[serde_as(as = "DefaultOnNull")]
    pub candidates_token_count: i64,
    #[serde_as(as = "DefaultOnNull")]
    pub total_token_count: i64,
    // the camel-case key answers while it holds a list, even an empty one, so both spellings stay readable
    pub prompt_tokens_details: Option<ResponseJson>,
    #[serde(rename = "prompt_tokens_details")]
    pub prompt_details_snake: Option<ResponseJson>,
    pub candidates_tokens_details: Option<ResponseJson>,
    #[serde(rename = "candidates_tokens_details")]
    pub candidate_details_snake: Option<ResponseJson>,
}

// ImageUsage is extra="allow" in LiteLLM, so the prompt and completion aliases ride beside its four fields
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImageGenerationUsage {
    pub input_tokens: i64,
    pub input_tokens_details: ImageInputTokens,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub prompt_tokens_details: ImageInputTokens,
    pub completion_tokens: i64,
    pub completion_tokens_details: ImageInputTokens,
    pub output_tokens_details: ImageInputTokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<i64>,
}

// a truthy usageMetadata is an object, and .get on any other shape is what LiteLLM crashes on
pub fn decode_image_usage(raw: &RawValue) -> Result<ImageUsageMetadata, StudioImageError> {
    let text = raw.get();
    if !text.trim_start().starts_with('{') {
        return Err(StudioImageError::UsageShape);
    }
    serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_str(text)).map_err(
        |error| StudioImageError::Shape {
            path: error.path().to_string(),
            source: error.into_inner(),
        },
    )
}

// transform_gemini_image_usage: summed TEXT and IMAGE counts, with every unexplained output token going to text
pub fn transform_gemini_image_usage(
    metadata: &ImageUsageMetadata,
) -> Result<ImageGenerationUsage, StudioImageError> {
    let input_tokens_details = modality_tokens(prompt_details(metadata), PROMPT_DETAILS)?;
    let candidates = candidate_details(metadata);
    let mut output_tokens_details = modality_tokens(candidates, CANDIDATE_DETAILS)?;
    let output_tokens = metadata.candidates_token_count;
    if candidates.is_empty() {
        output_tokens_details.image_tokens = output_tokens;
    } else {
        output_tokens_details.text_tokens = remainder_text(&output_tokens_details, output_tokens)?;
    }
    Ok(ImageGenerationUsage {
        input_tokens: metadata.prompt_token_count,
        input_tokens_details: input_tokens_details.clone(),
        output_tokens,
        total_tokens: metadata.total_token_count,
        prompt_tokens: metadata.prompt_token_count,
        prompt_tokens_details: input_tokens_details,
        completion_tokens: output_tokens,
        completion_tokens_details: output_tokens_details.clone(),
        output_tokens_details,
        web_search_requests: None,
    })
}

// a positive gap between the reported output tokens and the itemized ones is text, and overreported details stay as given
fn remainder_text(details: &ImageInputTokens, output_tokens: i64) -> Result<i64, StudioImageError> {
    let known = details
        .text_tokens
        .checked_add(details.image_tokens)
        .ok_or(StudioImageError::TokenOverflow {
            field: CANDIDATE_DETAILS,
        })?;
    if output_tokens <= known {
        return Ok(details.text_tokens);
    }
    let gap = output_tokens
        .checked_sub(known)
        .ok_or(StudioImageError::TokenOverflow {
            field: CANDIDATE_DETAILS,
        })?;
    details
        .text_tokens
        .checked_add(gap)
        .ok_or(StudioImageError::TokenOverflow {
            field: CANDIDATE_DETAILS,
        })
}

fn prompt_details(metadata: &ImageUsageMetadata) -> &[ResponseJson] {
    details_list(
        metadata.prompt_tokens_details.as_ref(),
        metadata.prompt_details_snake.as_ref(),
    )
}

fn candidate_details(metadata: &ImageUsageMetadata) -> &[ResponseJson] {
    details_list(
        metadata.candidates_tokens_details.as_ref(),
        metadata.candidate_details_snake.as_ref(),
    )
}

// _get_modality_token_details: the camel-case key answers while it holds a list, and null or a non-list falls through
fn details_list<'json>(
    camel: Option<&'json ResponseJson>,
    snake: Option<&'json ResponseJson>,
) -> &'json [ResponseJson] {
    if let Some(ResponseJson::List(entries)) = camel {
        return entries.as_slice();
    }
    match snake {
        Some(ResponseJson::List(entries)) => entries.as_slice(),
        _ => &[],
    }
}

// _sum_modality_token_details: a case-insensitive TEXT or IMAGE name counts, and anything else is skipped
fn modality_tokens(
    entries: &[ResponseJson],
    field: &'static str,
) -> Result<ImageInputTokens, StudioImageError> {
    let mut tokens = ImageInputTokens::default();
    for entry in entries {
        let ResponseJson::Object(fields) = entry else {
            continue;
        };
        let modality = match fields.get("modality") {
            Some(ResponseJson::Text(text)) => text.to_uppercase(),
            _ => continue,
        };
        let count = detail_token_count(fields);
        match modality.as_str() {
            "TEXT" => tokens.text_tokens = add(tokens.text_tokens, count, field)?,
            "IMAGE" => tokens.image_tokens = add(tokens.image_tokens, count, field)?,
            _ => {}
        }
    }
    Ok(tokens)
}

fn add(current: i64, count: i64, field: &'static str) -> Result<i64, StudioImageError> {
    current
        .checked_add(count)
        .ok_or(StudioImageError::TokenOverflow { field })
}

// _get_token_count: tokenCount answers first and then token_count, and only a JSON integer counts at all
fn detail_token_count(fields: &Extra) -> i64 {
    let Some(ResponseJson::Number(number)) = fields
        .get("tokenCount")
        .or_else(|| fields.get("token_count"))
    else {
        return 0;
    };
    number.as_i64().unwrap_or(0)
}
