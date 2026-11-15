use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHttpResponse, ImageResponse,
};
use indexmap::IndexMap;

use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::stable_diffusion::error::StableDiffusionError;

pub fn transform_reply(
    response: &mut ImageResponse,
    hidden_params: &mut HiddenParams,
    reply: &ImageHttpResponse,
) -> Result<(), StableDiffusionError> {
    let fields = decode_reply(reply)?;
    let images = appended_images(fields.get("images"))?;
    response.data.get_or_insert_with(Vec::new).extend(images);
    copy_hidden(&fields, hidden_params);
    Ok(())
}

// The shared Fal reply decode filters malformed images, so this reader decodes the object itself
fn decode_reply(reply: &ImageHttpResponse) -> Result<IndexMap<String, Json>, FalImageError> {
    if reply.body.len() > MAX_FAL_REPLY_BYTES {
        return Err(FalImageError::ReplyTooLarge {
            status: reply.status_code,
            headers: reply.headers.clone(),
            limit: MAX_FAL_REPLY_BYTES,
        });
    }
    let value: Json =
        serde_json::from_slice(&reply.body).map_err(|source| FalImageError::ReplyDecode {
            status: reply.status_code,
            headers: reply.headers.clone(),
            source,
        })?;
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(FalImageError::ReplyNotAnObject {
                status: reply.status_code,
                headers: reply.headers.clone(),
            })
        }
    }
}

fn appended_images(images: Option<&Json>) -> Result<Vec<ImageData>, StableDiffusionError> {
    let Some(Json::List(entries)) = images else {
        return Ok(Vec::new());
    };
    let mut appended = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let url = match entry {
            Json::Object(fields) => match fields.get("url") {
                Some(Json::Text(text)) => Some(text.clone()),
                Some(Json::Null) | None => None,
                Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                    return Err(StableDiffusionError::InvalidImageUrl { index });
                }
            },
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => continue,
        };
        appended.push(ImageData {
            b64_json: None,
            revised_prompt: None,
            url,
        });
    }
    Ok(appended)
}

fn copy_hidden(fields: &IndexMap<String, Json>, hidden_params: &mut HiddenParams) {
    for name in ["seed", "timings", "has_nsfw_concepts"] {
        if let Some(value) = fields.get(name) {
            hidden_params.insert(name.to_owned(), value.clone());
        }
    }
}
