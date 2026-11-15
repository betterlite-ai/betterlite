use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::api::fal::edit::error::FalEditError;
use crate::api::fal::edit::image_input::{EditImage, to_data_url};
use crate::api::fal::images::settings::FalImageSettings;

pub const EDIT_SUFFIX: &str = "/edit";

pub struct EditRequest<'value> {
    pub prompt: Option<&'value str>,
    pub images: &'value mut [Option<EditImage<'value>>],
    pub mask: &'value mut [Option<EditImage<'value>>],
    pub options: &'value ImageParams,
}

#[must_use]
pub fn complete_url(settings: &FalImageSettings, api_base: Option<&str>, model: &str) -> String {
    if model.ends_with(EDIT_SUFFIX) {
        settings.complete_url(api_base, model)
    } else {
        settings.complete_url(api_base, &format!("{model}{EDIT_SUFFIX}"))
    }
}

pub fn transform_image_edit_request(
    request: &mut EditRequest<'_>,
) -> Result<ImageEditForm, FalEditError> {
    let count = request
        .images
        .iter()
        .filter(|entry| entry.is_some())
        .count();
    if count == 0 {
        return Err(FalEditError::MissingImages);
    }
    let mask = mask_url(request.mask, request.options)?;
    let mut urls = Vec::with_capacity(count);
    for image in request.images.iter_mut().flatten() {
        urls.push(Json::Text(to_data_url(image)?));
    }
    let mut data = request_body(request.prompt);
    data.insert("image_urls".to_owned(), Json::List(urls));
    if let Some(url) = mask {
        data.insert("mask_url".to_owned(), Json::Text(url));
    }
    overlay_provider_options(&mut data, request.options);
    Ok(ImageEditForm {
        data,
        files: Vec::new(),
    })
}

pub(super) fn request_body(prompt: Option<&str>) -> ImageParams {
    let mut data = ImageParams::new();
    data.insert(
        "prompt".to_owned(),
        prompt.map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    data
}

pub(super) fn overlay_provider_options(data: &mut ImageParams, options: &ImageParams) {
    for (name, value) in options {
        if name != "mask" {
            data.insert(name.clone(), value.clone());
        }
    }
}

fn mask_url(
    mask: &mut [Option<EditImage<'_>>],
    options: &ImageParams,
) -> Result<Option<String>, FalEditError> {
    match mask {
        [Some(image), ..] => to_data_url(image).map(Some),
        [None, ..] => Ok(None),
        [] => option_mask(options),
    }
}

fn option_mask(options: &ImageParams) -> Result<Option<String>, FalEditError> {
    let Some(mask) = options.get("mask").map(first) else {
        return Ok(None);
    };
    if matches!(mask, Json::Null) {
        return Ok(None);
    }
    to_data_url(&mut json_image(mask)).map(Some)
}

fn first(value: &Json) -> &Json {
    match value {
        Json::List(entries) => entries.first().unwrap_or(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => value,
    }
}

#[must_use]
pub fn json_image(value: &Json) -> EditImage<'_> {
    match value {
        Json::Text(text) => EditImage::Text(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            EditImage::Other(value)
        }
    }
}
