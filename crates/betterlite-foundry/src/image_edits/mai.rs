use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::file::{DEFAULT_IMAGE_NAME, ImageEditFile, file_record};
use betterlite_openai::image_edits::request::ImageEditForm;

pub const MAI_EDIT_OPENAI_PARAMS: [&str; 4] = ["prompt", "image", "model", "n"];

pub struct MaiEditRequest<'data> {
    pub model: &'data str,
    pub prompt: Option<&'data str>,
    pub image: Option<&'data [Option<ImageEditFile>]>,
    pub options: &'data ImageParams,
}

// A prompt overrides the options, image and mask leave the form data, and only the first non-None listed image is uploaded
#[must_use]
pub fn transform_request(request: &MaiEditRequest<'_>) -> ImageEditForm {
    let mut data = ImageParams::new();
    data.insert(String::from("model"), Json::Text(request.model.to_owned()));
    for (name, value) in request.options {
        data.insert(name.clone(), value.clone());
    }
    if let Some(prompt) = request.prompt {
        data.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
    }
    data.shift_remove("image");
    data.shift_remove("mask");
    let mut files = Vec::new();
    if let Some(first) = request
        .image
        .and_then(|images| images.iter().flatten().next())
    {
        files.push(file_record("image", first, DEFAULT_IMAGE_NAME));
    }
    ImageEditForm { data, files }
}
