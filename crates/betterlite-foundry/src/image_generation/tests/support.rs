use crate::image_generation::error::FoundryImageError;
use crate::image_generation::params::{ImageParams, map_openai_params};

pub(super) fn object(text: &str) -> ImageParams {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn mapped(
    non_default: &str,
    optional: &str,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FoundryImageError> {
    map_openai_params(&object(non_default), &object(optional), model, drop_params)
}
