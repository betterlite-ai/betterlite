use betterlite_models::decisions::Json;

use super::error::ImageEditError;
use super::family::{ImageEditFamily, map_openai_params};
use super::file::{DEFAULT_IMAGE_NAME, DEFAULT_MASK_NAME, ImageEditFile, file_record};
use crate::api::image_contracts::records::{ImageFiles, ImageParams};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageEditRequest {
    pub model: String,
    pub prompt: Option<String>,
    pub image: Option<Vec<Option<ImageEditFile>>>,
    pub mask: Option<Vec<Option<ImageEditFile>>>,
    pub options: ImageParams,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageEditForm {
    pub data: ImageParams,
    pub files: ImageFiles,
}

// OpenAIImageEditConfig.transform_image_edit_request and DallE2ImageEditConfig's override
pub fn transform_image_edit_request(
    family: ImageEditFamily,
    request: &ImageEditRequest,
) -> Result<ImageEditForm, ImageEditError> {
    let mut data = ImageParams::new();
    data.insert(String::from("model"), Json::Text(request.model.clone()));
    data.extend(map_openai_params(&request.options));
    if let Some(prompt) = &request.prompt {
        data.insert(String::from("prompt"), Json::Text(prompt.clone()));
    }
    data.shift_remove("image");
    data.shift_remove("mask");
    let mut files = ImageFiles::new();
    if let Some(images) = &request.image {
        // DALL-E-2 counts the list before skipping None entries, so [image, None] still fails
        if family == ImageEditFamily::DallE2 && images.len() > 1 {
            return Err(ImageEditError::MultipleImages {
                model: request.model.clone(),
            });
        }
        for image in images.iter().flatten() {
            files.push(file_record(family.image_field(), image, DEFAULT_IMAGE_NAME));
        }
    }
    let mask = request
        .mask
        .as_ref()
        .and_then(|masks| masks.first())
        .and_then(Option::as_ref);
    if let Some(mask) = mask {
        files.push(file_record("mask", mask, DEFAULT_MASK_NAME));
    }
    Ok(ImageEditForm { data, files })
}
