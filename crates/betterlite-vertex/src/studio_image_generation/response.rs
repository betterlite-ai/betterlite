use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::{Extra, Json as ResponseJson};
use serde::Serialize;
use serde::de::Error as _;
use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use super::decode::decode_studio_image;
use super::error::StudioImageError;
use super::usage::{ImageGenerationUsage, decode_image_usage, transform_gemini_image_usage};
use crate::gemini_image_policy::generation::image_web_search_requests;
use crate::gemini_image_policy::policy::is_gemini_image_model;
use crate::vertex_gemini_images::decode::GoogleImageCandidate;
use crate::vertex_gemini_images::response::{GeneratedImage, ImageProviderFields};

// ImageObject and ImageUsage both allow extra fields, so an image keeps its thought signature and a usage its search count
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageGenerationResponse {
    pub data: Vec<GeneratedImage>,
    // an absent usageMetadata leaves usage away, and only a present usage takes the search count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ImageGenerationUsage>,
}

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_response(
    model: &str,
    reply: &UpstreamResponse,
) -> Result<ImageGenerationResponse, StudioImageError> {
    let decoded = decode_studio_image(reply)?;
    if is_gemini_image_model(model) {
        let candidates = decoded.candidates.as_deref().unwrap_or_default();
        let usage = gemini_usage(decoded.usage_metadata.as_deref(), search_count(candidates))?;
        return Ok(ImageGenerationResponse {
            data: gemini_images(candidates),
            usage,
        });
    }
    let predictions = decoded.predictions.as_deref().unwrap_or_default();
    // Imagen's branch reads no usage at all, so the reply's usageMetadata stays unread there
    Ok(ImageGenerationResponse {
        data: imagen_images(predictions)?,
        usage: None,
    })
}

fn gemini_usage(
    raw: Option<&RawValue>,
    search: Option<i64>,
) -> Result<Option<ImageGenerationUsage>, StudioImageError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut usage = transform_gemini_image_usage(&decode_image_usage(raw)?)?;
    usage.web_search_requests = search;
    Ok(Some(usage))
}

// every part carrying inlineData.data is an image, whatever its MIME type, and only a truthy signature rides along
fn gemini_images(candidates: &[GoogleImageCandidate]) -> Vec<GeneratedImage> {
    candidates
        .iter()
        .filter_map(|candidate| candidate.content.as_ref())
        .flat_map(|content| content.parts.iter())
        .filter_map(|part| {
            let encoded = part.inline_data.as_ref()?.data.as_ref()?;
            Some(GeneratedImage {
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
            })
        })
        .collect()
}

// Imagen appends one image per prediction, and a prediction without bytes keeps a null image
fn imagen_images(predictions: &[Extra]) -> Result<Vec<GeneratedImage>, StudioImageError> {
    let mut data = Vec::with_capacity(predictions.len());
    for (index, prediction) in predictions.iter().enumerate() {
        data.push(GeneratedImage {
            b64_json: prediction_bytes(prediction, index)?,
            url: None,
            revised_prompt: None,
            provider_specific_fields: None,
        });
    }
    Ok(data)
}

fn prediction_bytes(prediction: &Extra, index: usize) -> Result<Option<String>, StudioImageError> {
    match prediction.get("bytesBase64Encoded") {
        None | Some(ResponseJson::Null) => Ok(None),
        Some(ResponseJson::Text(encoded)) => Ok(Some(encoded.clone())),
        Some(_) => Err(StudioImageError::Shape {
            path: format!("predictions[{index}].bytesBase64Encoded"),
            source: JsonError::custom("the value is not a string"),
        }),
    }
}

// get_gemini_image_web_search_requests reads groundingMetadata off the candidates it is handed
fn search_count(candidates: &[GoogleImageCandidate]) -> Option<i64> {
    let entries = candidates
        .iter()
        .map(|candidate| {
            let mut fields = Extra::new();
            if let Some(metadata) = &candidate.grounding_metadata {
                fields.insert("groundingMetadata".to_owned(), metadata.clone());
            }
            ResponseJson::Object(fields)
        })
        .collect();
    image_web_search_requests(&Extra::from([(
        "candidates".to_owned(),
        ResponseJson::List(entries),
    )]))
}
