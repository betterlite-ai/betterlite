use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use indexmap::IndexMap;
use serde::Serialize;

use crate::api::fal::images::error::FalImageError;

pub const MAX_FAL_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FalImageProviderSpecificFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FalImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<FalImageProviderSpecificFields>,
}

#[must_use]
pub fn fal_images_to_image_objects(images: &Json) -> Vec<FalImage> {
    let Json::List(entries) = images else {
        return Vec::new();
    };
    entries.iter().filter_map(image_object).collect()
}

fn image_object(entry: &Json) -> Option<FalImage> {
    match entry {
        Json::Text(url) => Some(FalImage {
            url: Some(url.clone()),
            b64_json: None,
            provider_specific_fields: None,
        }),
        Json::Object(fields) => Some(FalImage {
            url: text_field(fields, "url"),
            b64_json: text_field(fields, "b64_json"),
            provider_specific_fields: provider_fields(fields),
        }),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}

fn text_field(fields: &IndexMap<String, Json>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn dimension(fields: &IndexMap<String, Json>, name: &str) -> Option<u64> {
    match fields.get(name) {
        Some(Json::Number(number)) => number.as_u64().filter(|value| *value > 0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn provider_fields(fields: &IndexMap<String, Json>) -> Option<FalImageProviderSpecificFields> {
    let provider = FalImageProviderSpecificFields {
        width: dimension(fields, "width"),
        height: dimension(fields, "height"),
        content_type: text_field(fields, "content_type"),
    };
    (provider != FalImageProviderSpecificFields::default()).then_some(provider)
}

// the permissive record view: the size and JSON checks and the object root, with every field left as sent
pub fn decode_fal_image_record(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<IndexMap<String, Json>, FalImageError> {
    if body.len() > MAX_FAL_REPLY_BYTES {
        return Err(FalImageError::ReplyTooLarge {
            status,
            headers: headers.clone(),
            limit: MAX_FAL_REPLY_BYTES,
        });
    }
    let value: Json =
        serde_json::from_slice(body).map_err(|source| FalImageError::ReplyDecode {
            status,
            headers: headers.clone(),
            source,
        })?;
    let Json::Object(fields) = value else {
        return Err(FalImageError::ReplyNotAnObject {
            status,
            headers: headers.clone(),
        });
    };
    Ok(fields)
}

pub fn decode_fal_image_reply(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<Vec<FalImage>, FalImageError> {
    let fields = decode_fal_image_record(status, headers, body)?;
    Ok(fields
        .get("images")
        .map_or_else(Vec::new, fal_images_to_image_objects))
}

pub fn append_generation_images(
    data: &mut Option<Vec<FalImage>>,
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<(), FalImageError> {
    let images = decode_fal_image_reply(status, headers, body)?;
    data.get_or_insert_with(Vec::new).extend(images);
    Ok(())
}
