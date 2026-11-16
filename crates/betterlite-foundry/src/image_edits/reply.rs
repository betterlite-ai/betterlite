use betterlite_openai::image_generation::decode::decode_image_reply;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::mai_images::reply::{ImageLogging, PostCall, decode_mai_image_reply};

use super::error::ImageEditError;

// The MAI edit reply normalizes a present usage through B554 and then B004 projects it; unlike generation, nothing
// here invents a response size or reads the request's width and height
pub fn decode_mai_edit_reply(
    body: &[u8],
    status_code: u16,
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageEditError> {
    let response = decode_mai_image_reply(body, status_code)?;
    project_reply(&response, created_seconds)
}

pub fn transform_mai_edit_response(
    body: &[u8],
    status_code: u16,
    created_seconds: i64,
    logging: &mut dyn ImageLogging,
) -> Result<ImageGenerationReply, ImageEditError> {
    let response = decode_mai_image_reply(body, status_code)?;
    let empty = Object::new();
    let api_key = SecretString::from(String::new());
    logging.post_call(PostCall {
        input: "",
        api_key: Some(&api_key),
        complete_input_dict: &empty,
        original_response: &response,
    });
    project_reply(&response, created_seconds)
}

fn project_reply(
    response: &Object,
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageEditError> {
    let bytes = serde_json::to_vec(response).map_err(ImageEditError::ReplyEncoding)?;
    Ok(decode_image_reply(&bytes, created_seconds)?)
}
