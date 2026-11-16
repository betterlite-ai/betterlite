use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::image_generation::flux::supported_params as generation_supported_params;
use crate::image_generation::params::map_openai_params;

use super::error::ImageEditError;
use super::image_source::{ImageSource, convert_image_to_base64};

pub const FLEX_REFERENCE_LIMIT: usize = 10;
pub const DEFAULT_REFERENCE_LIMIT: usize = 8;

pub struct Flux2EditRequest<'data> {
    pub model: &'data str,
    pub prompt: Option<&'data str>,
    pub images: Option<&'data mut [ImageSource]>,
    pub options: &'data ImageParams,
}

#[must_use]
pub fn supported_params(model: &str) -> &'static [&'static str] {
    generation_supported_params(model)
}

// B553's map_openai_params filters out None values, so an unsupported key sent as null is not reported
pub fn map_edit_params(
    options: &ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, ImageEditError> {
    let non_default: ImageParams = options
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Ok(map_openai_params(
        &non_default,
        &ImageParams::new(),
        model,
        drop_params,
    )?)
}

// Only a Python list expands into references, a None entry is an unsupported image, and the param overlay comes last
pub fn transform_request(request: Flux2EditRequest<'_>) -> Result<ImageEditForm, ImageEditError> {
    let prompt = request.prompt.ok_or(ImageEditError::Flux2PromptRequired)?;
    let images = request.images.ok_or(ImageEditError::Flux2ImageRequired)?;
    check_reference_count(&request.model.to_lowercase(), request.model, images.len())?;
    let mut data = ImageParams::new();
    data.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
    data.insert(String::from("model"), Json::Text(request.model.to_owned()));
    for (index, source) in images.iter_mut().enumerate() {
        let encoded = convert_image_to_base64(source)?;
        data.insert(reference_field(index), Json::Text(encoded));
    }
    for (name, value) in request.options {
        data.insert(name.clone(), value.clone());
    }
    Ok(ImageEditForm {
        data,
        files: ImageFiles::new(),
    })
}

fn check_reference_count(lowered: &str, model: &str, count: usize) -> Result<(), ImageEditError> {
    if count == 0 {
        return Err(ImageEditError::Flux2NoReferenceImages);
    }
    let limit = if lowered.contains("flex") {
        FLEX_REFERENCE_LIMIT
    } else {
        DEFAULT_REFERENCE_LIMIT
    };
    if count > limit {
        return Err(ImageEditError::Flux2TooManyReferenceImages {
            model: model.to_owned(),
            limit,
        });
    }
    Ok(())
}

// enumerate starts at 1, so the first reference is unsuffixed and the second is input_image_2
fn reference_field(index: usize) -> String {
    if index == 0 {
        String::from("input_image")
    } else {
        format!("input_image_{}", index + 1)
    }
}
