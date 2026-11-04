use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
};
use betterlite_openai::api::responses_objects::sdk::response::CachedTokensDetails as ResponseCachedTokensDetails;
use betterlite_openai::api::responses_objects::{
    InputTokensDetails, OutputTokensDetails, ResponseApiUsage,
};
use betterlite_openai::api::responses_shapes::{
    ImageGenerationCallType, OutputAnnotation, OutputCallStatus, OutputImageGenerationCall,
};
use serde::Deserialize;
use serde_json::Number;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct ImageItem {
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Deserialize)]
pub struct ImageUrl {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatAnnotation {
    pub r#type: Option<String>,
    pub url_citation: Option<UrlCitation>,
}

#[derive(Debug, Deserialize)]
pub struct UrlCitation {
    pub start_index: Option<i64>,
    pub end_index: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[path = "responses_output_details/tests.rs"]
mod tests;

pub fn image_results<F>(
    images: Option<&[ImageItem]>,
    finish_reason: Option<&str>,
    mut uuid: F,
) -> Vec<OutputImageGenerationCall>
where
    F: FnMut() -> String,
{
    let mut results = Vec::new();
    for image in images.unwrap_or_default() {
        let Some(url) = image
            .image_url
            .as_ref()
            .and_then(|image_url| image_url.url.as_deref())
        else {
            continue;
        };
        let image_result = url.strip_prefix("data:").map_or_else(
            || (!url.is_empty()).then(|| url.to_owned()),
            |data| data.split_once(',').map(|(_, suffix)| suffix.to_owned()),
        );
        if let Some(suffix_value) = image_result.filter(|suffix| !suffix.is_empty()) {
            results.push(OutputImageGenerationCall {
                r#type: ImageGenerationCallType::ImageGenerationCall,
                id: format!("ig_{}", uuid()),
                status: image_finish_status(finish_reason),
                result: Some(suffix_value),
                extra: BTreeMap::new(),
            });
        }
    }
    results
}

#[must_use]
pub fn image_finish_status(reason: Option<&str>) -> OutputCallStatus {
    match reason {
        Some("length") => OutputCallStatus::Incomplete,
        Some("content_filter" | "error") => OutputCallStatus::Failed,
        _ => OutputCallStatus::Completed,
    }
}

#[must_use]
pub fn citation_fields(annotations: Option<&[ChatAnnotation]>) -> Vec<OutputAnnotation> {
    let mut citations = Vec::new();
    for annotation in annotations.unwrap_or_default() {
        let Some(citation) = annotation
            .r#type
            .as_deref()
            .filter(|kind| *kind == "url_citation")
            .and(annotation.url_citation.as_ref())
        else {
            continue;
        };
        citations.push(OutputAnnotation {
            r#type: annotation.r#type.clone(),
            start_index: citation.start_index,
            end_index: citation.end_index,
            url: citation.url.clone(),
            title: citation.title.clone(),
            extra: BTreeMap::new(),
        });
    }
    citations
}

pub fn usage_details(usage_input: Option<&Usage>) -> ResponseApiUsage {
    let Some(usage) = usage_input else {
        return ResponseApiUsage {
            input_tokens: 0,
            input_tokens_details: None,
            output_tokens: 0,
            output_tokens_details: None,
            total_tokens: 0,
            cost: None,
            extra: BTreeMap::new(),
        };
    };
    ResponseApiUsage {
        input_tokens: usage.prompt_tokens,
        input_tokens_details: usage.prompt_tokens_details.as_ref().map(prompt_details),
        output_tokens: usage.completion_tokens,
        output_tokens_details: usage
            .completion_tokens_details
            .as_ref()
            .map(completion_details),
        total_tokens: usage.total_tokens,
        cost: match usage.numeric_extras.get("cost") {
            Some(UsageNumber::Decimal(cost)) => Some(*cost),
            _ => None,
        },
        extra: BTreeMap::new(),
    }
}

fn prompt_details(details: &PromptTokensDetails) -> InputTokensDetails {
    let mut extra = BTreeMap::new();
    if let Some(tokens) = details
        .cache_write_tokens
        .filter(|tokens| *tokens != 0)
        .or(details.cache_creation_tokens)
    {
        extra.insert(
            "cache_write_tokens".to_owned(),
            Json::Number(Number::from(tokens)),
        );
    }
    if let Some(count) = details.web_search_requests {
        extra.insert(
            "web_search_requests".to_owned(),
            Json::Number(Number::from(count)),
        );
    }
    if let Some(count) = details.google_maps_grounding_requests {
        extra.insert(
            "google_maps_grounding_requests".to_owned(),
            Json::Number(Number::from(count)),
        );
    }
    InputTokensDetails {
        audio_tokens: details.audio_tokens,
        cached_tokens: details.cached_tokens.unwrap_or(0),
        cached_tokens_details: details.cached_tokens_details.as_ref().map(cached_details),
        image_tokens: details.image_tokens,
        text_tokens: details.text_tokens,
        video_tokens: details.video_tokens,
        extra,
    }
}

const fn cached_details(details: &CachedTokensDetails) -> ResponseCachedTokensDetails {
    ResponseCachedTokensDetails {
        text_tokens: details.text_tokens,
        audio_tokens: details.audio_tokens,
        image_tokens: details.image_tokens,
    }
}

fn completion_details(details: &CompletionTokensDetails) -> OutputTokensDetails {
    let mut extra = BTreeMap::new();
    if let Some(image_tokens) = details.image_tokens {
        extra.insert(
            "image_tokens".to_owned(),
            Json::Number(Number::from(image_tokens)),
        );
    }
    OutputTokensDetails {
        audio_tokens: details.audio_tokens,
        reasoning_tokens: Some(details.reasoning_tokens.unwrap_or(0)),
        text_tokens: details.text_tokens,
        extra,
    }
}
