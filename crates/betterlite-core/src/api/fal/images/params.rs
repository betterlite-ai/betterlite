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

pub type SizeTranslation = fn(&Json) -> Result<(&'static str, Json), FalImageError>;

// an original key already in the options suppresses its translation; an unlisted key fails unless dropped
fn map_params<F>(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
    mut apply: F,
) -> Result<ImageParams, FalImageError>
where
    F: FnMut(&str, &Json, &mut ImageParams) -> Result<(), FalImageError>,
{
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
        apply(name, value, &mut mapped)?;
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
        |name, value, mapped| {
            mapped.insert(name.to_owned(), value.clone());
            Ok(())
        },
    )
}

// the pinned Bria and Recraft classes advertise n and response_format but never send them, and rename size
pub fn map_url_only_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
    translate_size: SizeTranslation,
) -> Result<ImageParams, FalImageError> {
    map_params(
        non_default_params,
        optional_params,
        model,
        drop_params,
        |name, value, mapped| {
            if name == "size" {
                let (destination, translated) = translate_size(value)?;
                mapped.insert(destination.to_owned(), translated);
            }
            Ok(())
        },
    )
}
