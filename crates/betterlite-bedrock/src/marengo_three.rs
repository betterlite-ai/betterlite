pub mod error;
pub mod fields;
mod inputs;
mod integer_text;
pub mod options;
pub mod params;
pub mod request;
mod tests;

use error::{Marengo3Error, bad_request};
use inputs::{media_source, multi_input, text_image_input, timed_media_input};
use options::InputType;
use params::{Marengo3Params, RawParams};
use request::{ImageInput, Marengo3Request, RequestBody, TextInput};

pub const MARENGO_3_MODEL_MARKER: &str = "marengo-embed-3-";

#[must_use]
pub fn is_marengo_3_model(model: Option<&str>) -> bool {
    let Some(name) = model else { return false };
    name.contains(MARENGO_3_MODEL_MARKER)
}

pub fn build_marengo_3_request(
    input: &str,
    inference_params: &RawParams,
    drop_params: bool,
) -> Result<Marengo3Request, Marengo3Error> {
    let params = Marengo3Params::validate(inference_params)?;
    let input_type = params.resolved_input_type();
    if !drop_params {
        reject_unsupported(&params, input_type)?;
    }
    let body = request_body(input, input_type, &params)?;
    Ok(Marengo3Request {
        inference_id: params.inference_id,
        body,
    })
}

fn reject_unsupported(params: &Marengo3Params, input_type: InputType) -> Result<(), Marengo3Error> {
    reject_given(
        &params.given_2_7_only,
        "Marengo 3.0 does not accept the Marengo 2.7 parameters",
    )?;
    if matches!(input_type, InputType::Video | InputType::Audio) {
        return Ok(());
    }
    reject_given(
        &params.timed.given(),
        &format!("Input type '{}' does not accept", input_type.name()),
    )
}

fn request_body(
    input: &str,
    input_type: InputType,
    params: &Marengo3Params,
) -> Result<RequestBody, Marengo3Error> {
    Ok(match input_type {
        InputType::Text => RequestBody::Text {
            text: TextInput {
                input_text: input.to_owned(),
            },
        },
        InputType::Image => RequestBody::Image {
            image: ImageInput {
                media_source: media_source(input, params.bucket_owner.as_deref())?,
            },
        },
        InputType::Video => RequestBody::Video {
            video: timed_media_input(input, params)?,
        },
        InputType::Audio => RequestBody::Audio {
            audio: timed_media_input(input, params)?,
        },
        InputType::TextImage => RequestBody::TextImage {
            text_image: text_image_input(input, params)?,
        },
        InputType::MultiInput => RequestBody::MultiInput {
            multi_input: multi_input(input, params)?,
        },
    })
}

fn reject_given(given: &[&str], reason: &str) -> Result<(), Marengo3Error> {
    if given.is_empty() {
        return Ok(());
    }
    Err(bad_request(format!(
        "{reason} {}; set drop_params to drop them",
        given.join(", ")
    )))
}
