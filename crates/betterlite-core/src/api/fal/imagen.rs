pub mod size;
mod tests;

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};

use crate::api::fal::imagen::size::map_aspect_ratio;
use crate::api::fal::images::decode::{FalImage, decode_fal_image_record};
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::image_url;
use crate::api::fal::images::params::GENERIC_OPENAI_PARAMS;

// the fast and ultra variants named in LiteLLM's docstring still resolve to this one endpoint
pub const ENDPOINT: &str = "fal-ai/imagen4/preview";

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = GENERIC_OPENAI_PARAMS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImagenImageResponse {
    pub data: Vec<FalImage>,
    pub hidden_params: HiddenParams,
}

// each original key already in the options suppresses its own translation, and response_format is dropped
pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FalImageError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        if mapped.contains_key(name) {
            continue;
        }
        match name.as_str() {
            "n" => {
                mapped.insert("num_images".to_owned(), value.clone());
            }
            "response_format" => {}
            "size" => {
                let Json::Text(size) = value else {
                    return Err(FalImageError::SizeNotText);
                };
                mapped.insert(
                    "aspect_ratio".to_owned(),
                    Json::Text(map_aspect_ratio(size).to_owned()),
                );
            }
            _ if drop_params => {}
            _ => {
                return Err(FalImageError::UnsupportedParameters {
                    parameters: vec![name.clone()],
                    model: model.to_owned(),
                });
            }
        }
    }
    Ok(mapped)
}

// only urls survive: a text member is the url, a map member keeps its url, and the seed is copied when present
pub fn project_response(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
    response: &mut ImagenImageResponse,
) -> Result<(), FalImageError> {
    let reply = decode_fal_image_record(status, headers, body)?;
    if let Some(Json::List(images)) = reply.get("images") {
        for image in images {
            let url = match image {
                Json::Text(url) => Some(url.clone()),
                Json::Object(values) => image_url(values, ENDPOINT)?,
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => continue,
            };
            response.data.push(FalImage {
                url,
                b64_json: None,
                provider_specific_fields: None,
            });
        }
    }
    if let Some(seed) = reply.get("seed") {
        response
            .hidden_params
            .insert("seed".to_owned(), seed.clone());
    }
    Ok(())
}
