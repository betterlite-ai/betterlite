use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as ResponseJson;
use betterlite_openai::api::image_contracts::records::{ImageInputTokens, ImageUsage};
use indexmap::IndexMap;
use serde::Serialize;

use super::decode::{
    GoogleImageCandidate, ImageUsageMetadata, decode_google_image, decode_image_usage,
};
use super::error::GeminiImageError;
use crate::gemini_image_policy::generation::image_web_search_requests;

// ImageObject with the provider_specific_fields a Gemini reply can carry
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GeneratedImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<ImageProviderFields>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageProviderFields {
    pub thought_signature: String,
}

// ImageUsage is extra="allow" in LiteLLM, so the search count rides beside its four declared fields
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageGenerationUsage {
    #[serde(flatten)]
    pub usage: ImageUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageGenerationResponse {
    pub data: Vec<GeneratedImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ImageGenerationUsage>,
}

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_response(
    reply: &UpstreamResponse,
) -> Result<ImageGenerationResponse, GeminiImageError> {
    let decoded = decode_google_image(reply)?;
    let candidates = decoded.candidates.as_deref().unwrap_or_default();
    let data = generated_images(candidates);
    // absent or empty usage leaves usage away, and only a present usage takes the search count
    let usage_metadata = decoded
        .usage_metadata
        .as_deref()
        .map(decode_image_usage)
        .transpose()?
        .flatten();
    let usage = match usage_metadata {
        Some(metadata) => Some(ImageGenerationUsage {
            // ast-grep-ignore: no-long-names
            usage: transform_image_generation_usage(&metadata)?,
            web_search_requests: web_search_requests(candidates),
        }),
        None => None,
    };
    Ok(ImageGenerationResponse { data, usage })
}

// every part carrying inlineData.data is an image, whatever its MIME type
fn generated_images(candidates: &[GoogleImageCandidate]) -> Vec<GeneratedImage> {
    let mut data = Vec::new();
    for candidate in candidates {
        let Some(content) = &candidate.content else {
            continue;
        };
        for part in &content.parts {
            let Some(encoded) = part
                .inline_data
                .as_ref()
                .and_then(|inline| inline.data.as_ref())
            else {
                continue;
            };
            data.push(GeneratedImage {
                b64_json: Some(encoded.clone()),
                url: None,
                revised_prompt: None,
                provider_specific_fields: part
                    .thought_signature
                    .as_deref()
                    .filter(|signature| !signature.is_empty())
                    .map(|signature| ImageProviderFields {
                        thought_signature: signature.to_owned(),
                    }),
            });
        }
    }
    data
}

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_usage(
    usage: &ImageUsageMetadata,
) -> Result<ImageUsage, GeminiImageError> {
    let mut image_tokens = 0_i64;
    let mut text_tokens = 0_i64;
    for detail in &usage.prompt_tokens_details {
        if detail.modality.is_empty() {
            continue;
        }
        match detail.modality.as_str() {
            "TEXT" => {
                text_tokens = text_tokens.checked_add(detail.token_count).ok_or(
                    GeminiImageError::TokenOverflow {
                        field: "promptTokensDetails",
                    },
                )?;
            }
            "IMAGE" => {
                image_tokens = image_tokens.checked_add(detail.token_count).ok_or(
                    GeminiImageError::TokenOverflow {
                        field: "promptTokensDetails",
                    },
                )?;
            }
            _ => {}
        }
    }
    Ok(ImageUsage {
        input_tokens: usage.prompt_token_count,
        input_tokens_details: ImageInputTokens {
            image_tokens,
            text_tokens,
        },
        output_tokens: usage.candidates_token_count,
        total_tokens: usage.total_token_count,
    })
}

// get_gemini_image_web_search_requests reads candidates and groundingMetadata from the reply object
fn web_search_requests(candidates: &[GoogleImageCandidate]) -> Option<i64> {
    let entries = candidates
        .iter()
        .map(|candidate| {
            let mut fields = IndexMap::new();
            if let Some(metadata) = &candidate.grounding_metadata {
                fields.insert("groundingMetadata".to_owned(), metadata.clone());
            }
            ResponseJson::Object(fields)
        })
        .collect();
    image_web_search_requests(&IndexMap::from([(
        "candidates".to_owned(),
        ResponseJson::List(entries),
    )]))
}
