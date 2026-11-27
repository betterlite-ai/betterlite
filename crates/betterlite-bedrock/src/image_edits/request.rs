use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::error::ImageEditError;
use super::plan::{EditCall, EditImage, EditPlan};
use crate::canvas_edits::request::{
    CanvasEditRequestInput, transform_image_edit_request as canvas_request,
};
use crate::canvas_edits::selection::{ImageEditConfig, image_edit_config_for_model};
use crate::canvas_edits::types::{CanvasEditOptions, CanvasImageInput};
use crate::stability_edits::request::transform_image_edit_request as stability_request;
use crate::stability_edits::types::{EditOptions, EditValue, ImageInput};

pub fn request_body(
    call: &EditCall<'_>,
    plan: &EditPlan<'_, '_>,
) -> Result<ImageParams, ImageEditError> {
    let config = image_edit_config_for_model(
        call.model,
        plan.model_cost,
        plan.resolution,
        &mut ModelCostKeys::default(),
    )?;
    let image = call.images.first();
    match config {
        ImageEditConfig::Stability => stability_body(call, image),
        ImageEditConfig::NovaCanvas => canvas_body(call, image),
    }
}

// LiteLLM hands the first image only to the transform, whatever the rest of the list holds.
fn stability_body(
    call: &EditCall<'_>,
    image: Option<&EditImage>,
) -> Result<ImageParams, ImageEditError> {
    let options: EditOptions = call
        .optional_params
        .iter()
        .map(|(name, value)| (name.clone(), EditValue::from(value.clone())))
        .collect();
    let input = image.map(stability_image);
    let request = stability_request(call.model, call.prompt, input.as_ref(), &options)?;
    match request.body {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ImageEditError::BodyShape)
        }
    }
}

// The Nova Canvas transform returns its typed body; the request sends the object it serializes to.
fn canvas_body(
    call: &EditCall<'_>,
    image: Option<&EditImage>,
) -> Result<ImageParams, ImageEditError> {
    let options = CanvasEditOptions::from_params(&call.optional_params)?;
    let input = image.map(canvas_image);
    let request = canvas_request(&CanvasEditRequestInput {
        prompt: call.prompt,
        image: input.as_ref(),
        options: &options,
    })?;
    let text = serde_json::to_string(&request.body)?;
    Ok(serde_json::from_str::<ImageParams>(&text)?)
}

fn stability_image(image: &EditImage) -> ImageInput {
    match image {
        EditImage::Bytes(content) => ImageInput::Bytes(content.clone()),
        EditImage::Text(text) => ImageInput::Text(text.clone()),
    }
}

fn canvas_image(image: &EditImage) -> CanvasImageInput {
    match image {
        EditImage::Bytes(content) => CanvasImageInput::Bytes(content.clone()),
        EditImage::Text(text) => CanvasImageInput::Text(text.clone()),
    }
}
