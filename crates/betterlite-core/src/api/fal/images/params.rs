use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::images::error::FalImageError;

pub const GENERIC_OPENAI_PARAMS: [&str; 3] = ["n", "response_format", "size"];

#[must_use]
pub fn image_generation_body(prompt: &str, optional_params: ImageParams) -> ImageParams {
    let mut body = ImageParams::new();
    body.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    for (name, value) in optional_params {
        body.insert(name, value);
    }
    body
}

pub(crate) type SizeTranslation = fn(&Json) -> Result<(&'static str, Json), FalImageError>;

// the pinned Bria and Recraft classes advertise n and response_format but send neither, and rename size
#[derive(Clone, Copy)]
pub(crate) enum SizeMapping {
    Verbatim,
    UrlOnly(SizeTranslation),
}

// an original key already in the options suppresses its translation; an unlisted key fails unless dropped
pub(crate) fn map_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
    sizes: SizeMapping,
) -> Result<ImageParams, FalImageError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        if mapped.contains_key(name) {
            continue;
        }
        if !GENERIC_OPENAI_PARAMS.contains(&name.as_str()) {
            if drop_params {
                continue;
            }
            return Err(FalImageError::UnsupportedParameters {
                parameters: vec![name.clone()],
                model: model.to_owned(),
            });
        }
        match sizes {
            SizeMapping::Verbatim => {
                mapped.insert(name.clone(), value.clone());
            }
            SizeMapping::UrlOnly(translate_size) => {
                if name == "size" {
                    let (destination, translated) = translate_size(value)?;
                    mapped.insert(destination.to_owned(), translated);
                }
            }
        }
    }
    Ok(mapped)
}

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FalImageError> {
    map_params(
        non_default_params,
        optional_params,
        model,
        drop_params,
        SizeMapping::Verbatim,
    )
}
