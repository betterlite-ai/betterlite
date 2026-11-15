use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use betterlite_openai::image_generation::reply::ImageDatum;
use indexmap::IndexMap;
use serde::Serialize;

use crate::api::fal::edit::error::FalEditError;
use crate::api::fal::images::decode::{
    FalImage, FalImageProviderSpecificFields, decode_fal_image_reply,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditResponse {
    pub created: i64,
    pub data: Vec<ImageDatum>,
}

pub fn transform_image_edit_response(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
    created_seconds: i64,
) -> Result<EditResponse, FalEditError> {
    let images = decode_fal_image_reply(status, headers, body)?;
    Ok(EditResponse {
        created: created_seconds,
        data: images.into_iter().map(image_datum).collect(),
    })
}

fn image_datum(image: FalImage) -> ImageDatum {
    ImageDatum {
        b64_json: image.b64_json,
        url: image.url,
        revised_prompt: None,
        provider_specific_fields: image.provider_specific_fields.map(provider_fields),
    }
}

fn provider_fields(fields: FalImageProviderSpecificFields) -> IndexMap<String, Json> {
    let mut provider = IndexMap::new();
    if let Some(width) = fields.width {
        provider.insert(
            "width".to_owned(),
            Json::Number(serde_json::Number::from(width)),
        );
    }
    if let Some(height) = fields.height {
        provider.insert(
            "height".to_owned(),
            Json::Number(serde_json::Number::from(height)),
        );
    }
    if let Some(content_type) = fields.content_type {
        provider.insert("content_type".to_owned(), Json::Text(content_type));
    }
    provider
}
