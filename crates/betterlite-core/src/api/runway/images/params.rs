use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::error::RunwayImageError;
use super::{DEFAULT_MODEL, DEFAULT_RATIO, SUPPORTED_OPENAI_PARAMS};

const SIZE_RATIOS: [(&str, &str); 5] = [
    ("1024x1024", "1024:1024"),
    ("1792x1024", "1792:1024"),
    ("1024x1792", "1024:1792"),
    ("1920x1080", "1920:1080"),
    ("1080x1920", "1080:1920"),
];
const EXCLUDED_PARAMS: [&str; 1] = ["size"];

fn ratio_for(size: &Json) -> Result<&'static str, RunwayImageError> {
    match size {
        Json::Text(text) => Ok(SIZE_RATIOS
            .iter()
            .find(|(preset, _)| *preset == text.as_str())
            .map_or(DEFAULT_RATIO, |(_, ratio)| *ratio)),
        Json::Null => Ok(DEFAULT_RATIO),
        _ => Err(RunwayImageError::InvalidSize),
    }
}

pub fn map_openai_params(
    non_default_params: &ImageParams,
    mut optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, RunwayImageError> {
    if let Some(size) = non_default_params.get("size") {
        optional_params.insert("ratio".to_owned(), Json::Text(ratio_for(size)?.to_owned()));
    }
    for (parameter, value) in non_default_params {
        if optional_params.contains_key(parameter) {
            continue;
        }
        if SUPPORTED_OPENAI_PARAMS.contains(&parameter.as_str()) {
            optional_params.insert(parameter.clone(), value.clone());
        } else if !drop_params {
            return Err(RunwayImageError::UnsupportedParameter {
                parameter: parameter.clone(),
                model: model.to_owned(),
            });
        }
    }
    Ok(optional_params)
}

#[must_use]
pub fn request_body(model: &str, prompt: &str, optional_params: &ImageParams) -> ImageParams {
    let mut body = ImageParams::new();
    let name = if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    };
    body.insert("model".to_owned(), Json::Text(name.to_owned()));
    body.insert("promptText".to_owned(), Json::Text(prompt.to_owned()));
    let ratio = optional_params
        .get("ratio")
        .cloned()
        .unwrap_or_else(|| Json::Text(DEFAULT_RATIO.to_owned()));
    body.insert("ratio".to_owned(), ratio);
    for (parameter, value) in optional_params {
        if !body.contains_key(parameter) && !EXCLUDED_PARAMS.contains(&parameter.as_str()) {
            body.insert(parameter.clone(), value.clone());
        }
    }
    body
}
