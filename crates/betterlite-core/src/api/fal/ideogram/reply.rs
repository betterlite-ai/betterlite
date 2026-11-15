use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHttpResponse, ImageResponse,
};
use indexmap::IndexMap;

use crate::api::fal::ideogram::error::IdeogramError;
use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;

pub fn transform_reply(
    response: &mut ImageResponse,
    hidden_params: &mut HiddenParams,
    reply: &ImageHttpResponse,
) -> Result<(), IdeogramError> {
    let fields = decode_reply(reply)?;
    let images = appended_images(fields.get("images"))?;
    response.data.get_or_insert_with(Vec::new).extend(images);
    if let Some(seed) = fields.get("seed") {
        hidden_params.insert("seed".to_owned(), seed.clone());
    }
    Ok(())
}

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

fn appended_images(images: Option<&Json>) -> Result<Vec<ImageData>, IdeogramError> {
    let Some(Json::List(entries)) = images else {
        return Ok(Vec::new());
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| image_object(entry, index))
        .collect()
}

fn image_object(entry: &Json, index: usize) -> Result<ImageData, IdeogramError> {
    let url = match entry {
        Json::Object(fields) => match fields.get("url") {
            Some(Json::Text(text)) => Some(text.clone()),
            Some(Json::Null) | None => None,
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(IdeogramError::InvalidImageUrl { index });
            }
        },
        Json::Text(text) => Some(text.clone()),
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            return Err(IdeogramError::InvalidImageUrl { index });
        }
    };
    Ok(ImageData {
        b64_json: None,
        revised_prompt: None,
        url,
    })
}
