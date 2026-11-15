pub mod size;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageParams, ImageResponse,
};

use crate::api::fal::bria::size::translate_size;
use crate::api::fal::images::UrlOnlyImageError;
use crate::api::fal::images::decode_url_only_reply;
use crate::api::fal::images::image_url;
use crate::api::fal::images::map_url_only_params;

pub const ENDPOINT: &str = "bria/text-to-image/3.2";

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, UrlOnlyImageError> {
    map_url_only_params(
        non_default_params,
        optional_params,
        model,
        drop_params,
        translate_size,
    )
}

// the singular image must be a non-empty object; a plural images field and every other type do nothing
pub fn append_singular_image(
    model_response: &mut ImageResponse,
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<(), UrlOnlyImageError> {
    let reply = decode_url_only_reply(status, headers, body)?;
    let data = model_response.data.get_or_insert_with(Vec::new);
    let Some(Json::Object(image)) = reply.get("image") else {
        return Ok(());
    };
    if image.is_empty() {
        return Ok(());
    }
    let url = image_url(image, ENDPOINT)?;
    data.push(ImageData {
        b64_json: None,
        revised_prompt: None,
        url,
    });
    Ok(())
}
