pub mod size;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageParams, ImageResponse,
};

use crate::api::fal::images::UrlOnlyImageError;
use crate::api::fal::images::decode_url_only_reply;
use crate::api::fal::images::image_url;
use crate::api::fal::images::map_url_only_params;
use crate::api::fal::recraft::size::translate_size;

pub const ENDPOINT: &str = "fal-ai/recraft/v3/text-to-image";

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

// every list member is a url object; a map member keeps only its url and a text member is itself the url
pub fn append_url_images(
    model_response: &mut ImageResponse,
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<(), UrlOnlyImageError> {
    let reply = decode_url_only_reply(status, headers, body)?;
    let data = model_response.data.get_or_insert_with(Vec::new);
    let Some(Json::List(images)) = reply.get("images") else {
        return Ok(());
    };
    for image in images {
        match image {
            Json::Text(url) => data.push(ImageData {
                b64_json: None,
                revised_prompt: None,
                url: Some(url.clone()),
            }),
            Json::Object(values) => {
                let url = image_url(values, ENDPOINT)?;
                data.push(ImageData {
                    b64_json: None,
                    revised_prompt: None,
                    url,
                });
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
        }
    }
    Ok(())
}
