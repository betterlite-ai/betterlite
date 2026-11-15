use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHttpResponse, ImageResponse,
};

use crate::api::fal::ideogram::error::IdeogramError;
use crate::api::fal::images::decode::decode_fal_image_object;

pub fn transform_reply(
    response: &mut ImageResponse,
    hidden_params: &mut HiddenParams,
    reply: &ImageHttpResponse,
) -> Result<(), IdeogramError> {
    let fields = decode_fal_image_object(reply.status_code, &reply.headers, &reply.body)?;
    let images = appended_images(fields.get("images"))?;
    response.data.get_or_insert_with(Vec::new).extend(images);
    if let Some(seed) = fields.get("seed") {
        hidden_params.insert("seed".to_owned(), seed.clone());
    }
    Ok(())
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
