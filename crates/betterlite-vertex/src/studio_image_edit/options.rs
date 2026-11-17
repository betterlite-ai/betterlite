use betterlite_prompts::arguments::json::Object;

use super::error::StudioImageEditError;
use crate::gemini_image_policy::policy::{ImageParamMapping, map_image_params};

// get_supported_openai_params: the edit dialect advertises the same three names for every model
pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["n", "size", "imageConfig"];

// map_openai_params: B1334's mapping with imageConfig string parsing, since an edit request carries it as text
pub fn map_openai_params(params: &Object, model: &str) -> Result<Object, StudioImageEditError> {
    let already_mapped = Object::new();
    let mapping = ImageParamMapping {
        params,
        model,
        supported_params: &SUPPORTED_OPENAI_PARAMS,
        optional_params: &already_mapped,
        image_size_support: None,
        parse_image_config_string: true,
    };
    map_image_params(&mapping).map_err(StudioImageEditError::Policy)
}
