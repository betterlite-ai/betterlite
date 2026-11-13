use std::time::Duration;

use betterlite_models::decisions::Json;
use secrecy::SecretString;

use self::client::{SharedSession, SuppliedClient};
use crate::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};

pub mod call;
pub mod client;
pub mod error;
pub mod logging;
pub mod request;
pub mod transform;
pub mod transport;
pub mod wire;

mod tests;

pub const PROVIDER: &str = "openai";

const MAX_RETRIES: i64 = 2;

// LiteLLM reads only litellm_params.async_call here; every other key belongs to the layers around this call
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageVariationParams {
    pub async_call: bool,
}

#[derive(Debug, Clone)]
pub struct ImageVariationRequest {
    pub model: Option<String>,
    pub image: ImageFile,
    pub optional_params: ImageParams,
    pub headers: Option<ImageHeaders>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<Duration>,
    pub params: ImageVariationParams,
    pub client: Option<SuppliedClient>,
    pub shared_session: Option<SharedSession>,
    pub config: Option<OpenAiImageVariationConfig>,
    pub boundary: String,
    pub now_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiImageVariationConfig;

#[derive(Debug, Clone)]
pub struct ImageVariationData {
    pub image: ImageFile,
    pub options: ImageParams,
}

// LiteLLM's OpenAIImageVariationConfig: the four optional params, and the image beside them
impl OpenAiImageVariationConfig {
    pub const SUPPORTED_PARAMS: [&'static str; 4] = ["n", "size", "response_format", "user"];

    // map_openai_params copies every entry, supported or not, and a new value wins over an existing one
    pub fn map_params(non_default: &ImageParams, optional_params: &mut ImageParams) {
        for (name, value) in non_default {
            optional_params.insert(name.clone(), value.clone());
        }
    }

    // transform_request_image_variation: {"image": supplied image, **options}; the model is never added
    pub fn request_data(
        image: ImageFile,
        optional_params: ImageParams,
    ) -> Option<ImageVariationData> {
        if optional_params.contains_key("image") {
            return None;
        }
        Some(ImageVariationData {
            image,
            options: optional_params,
        })
    }
}

// max_retries is an SDK setting, so it never reaches the image part
pub(super) fn take_max_retries(
    optional_params: &mut ImageParams,
) -> Result<i64, error::ImageVariationSource> {
    match optional_params.shift_remove("max_retries") {
        None => Ok(MAX_RETRIES),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(error::ImageVariationSource::MaxRetries),
        Some(_) => Err(error::ImageVariationSource::MaxRetries),
    }
}
