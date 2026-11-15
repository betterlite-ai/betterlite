use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::api::fal::edit::error::FalEditError;
use crate::api::fal::edit::image_input::{EditImage, to_data_url};
use crate::api::fal::edit::transformation::{EditRequest, overlay_provider_options, request_body};
use crate::api::fal::gpt_image::size::map_gpt_image_size;

pub const DEPTH_ENDPOINT: &str = "fal-ai/flux-lora-depth";
pub const OPENAI_PARAMS: [&str; 2] = ["n", "size"];
pub const PARAM_TRANSLATION: [(&str, &str); 2] = [("n", "num_images"), ("size", "image_size")];

#[must_use]
pub fn map_openai_params(options: &ImageParams) -> ImageParams {
    let mut mapped = ImageParams::new();
    for (name, value) in options {
        if matches!(value, Json::Null) {
            continue;
        }
        let Some((_, destination)) = PARAM_TRANSLATION
            .iter()
            .find(|(source, _)| *source == name.as_str())
        else {
            continue;
        };
        let mapped_value = if name == "size" {
            map_gpt_image_size(value)
        } else {
            value.clone()
        };
        mapped.insert((*destination).to_owned(), mapped_value);
    }
    mapped
}

pub fn transform_image_edit_request(
    request: &EditRequest<'_>,
) -> Result<ImageEditForm, FalEditError> {
    let images: Vec<&EditImage<'_>> = request.images.iter().flatten().collect();
    let Some(control) = images.first().copied() else {
        return Err(FalEditError::MissingImages);
    };
    if images.len() > 1 {
        return Err(FalEditError::ControlImages {
            endpoint: DEPTH_ENDPOINT.to_owned(),
        });
    }
    let mut data = request_body(request.prompt);
    data.insert("image_url".to_owned(), Json::Text(to_data_url(control)?));
    overlay_provider_options(&mut data, request.options);
    Ok(ImageEditForm {
        data,
        files: Vec::new(),
    })
}
