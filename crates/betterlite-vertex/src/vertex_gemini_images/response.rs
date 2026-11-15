use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as ResponseJson;
use betterlite_openai::api::image_contracts::records::{ImageInputTokens, ImageUsage};
use indexmap::IndexMap;
use serde::Serialize;

use super::decode::{GoogleImageCandidate, decode_google_image};
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

pub fn transform_image_generation_response(
    reply: &UpstreamResponse,
) -> Result<ImageGenerationResponse, GeminiImageError> {
    let decoded = decode_google_image(reply)?;
    let candidates = decoded.candidates.as_deref().unwrap_or_default();
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
    // an absent or empty usageMetadata leaves usage away; a non-object value is a typed error, and
    // only a present usage takes the search count
    let usage = decoded
        .usage_metadata
        .as_ref()
        .filter(|metadata| !matches!(metadata, ResponseJson::Object(fields) if fields.is_empty()))
        .map(transform_image_generation_usage)
        .transpose()?
        .map(|usage| ImageGenerationUsage {
            usage,
            web_search_requests: web_search_requests(candidates),
        });
    Ok(ImageGenerationResponse { data, usage })
}

pub fn transform_image_generation_usage(
    usage: &ResponseJson,
) -> Result<ImageUsage, GeminiImageError> {
    let ResponseJson::Object(fields) = usage else {
        return Err(GeminiImageError::UsageShape);
    };
    let mut image_tokens = 0_i64;
    let mut text_tokens = 0_i64;
    if let Some(ResponseJson::List(details)) = fields.get("promptTokensDetails") {
        for detail in details {
            let ResponseJson::Object(detail) = detail else {
                continue;
            };
            let Some(ResponseJson::Text(modality)) =
                detail.get("modality").filter(|value| value.is_truthy())
            else {
                continue;
            };
            match modality.as_str() {
                "TEXT" => {
                    let count = token_count(detail, "promptTokensDetails")?;
                    text_tokens =
                        text_tokens
                            .checked_add(count)
                            .ok_or(GeminiImageError::TokenOverflow {
                                field: "promptTokensDetails",
                            })?;
                }
                "IMAGE" => {
                    let count = token_count(detail, "promptTokensDetails")?;
                    image_tokens =
                        image_tokens
                            .checked_add(count)
                            .ok_or(GeminiImageError::TokenOverflow {
                                field: "promptTokensDetails",
                            })?;
                }
                _ => {}
            }
        }
    }
    Ok(ImageUsage {
        input_tokens: token_count(fields, "promptTokenCount")?,
        input_tokens_details: ImageInputTokens {
            image_tokens,
            text_tokens,
        },
        output_tokens: token_count(fields, "candidatesTokenCount")?,
        total_tokens: token_count(fields, "totalTokenCount")?,
    })
}

// usage.get(name, 0): an absent or null count is zero, and a non-integer one fails the typed usage
fn token_count(
    fields: &IndexMap<String, ResponseJson>,
    field: &'static str,
) -> Result<i64, GeminiImageError> {
    match fields.get(field) {
        None | Some(ResponseJson::Null) => Ok(0),
        Some(ResponseJson::Number(number)) => number
            .as_i64()
            .ok_or(GeminiImageError::TokenCount { field }),
        Some(
            ResponseJson::Bool(_)
            | ResponseJson::Text(_)
            | ResponseJson::List(_)
            | ResponseJson::Object(_),
        ) => Err(GeminiImageError::TokenCount { field }),
    }
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
