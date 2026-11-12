use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use super::decode::decode_image_reply;
use super::defaults::apply_family_defaults;
use super::family::ImageFamily;
use super::reply::{ImageGenerationReply, ImageReplyError};

#[derive(Debug, Error)]
pub enum ImageTransformError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Reply(#[from] ImageReplyError),
}

#[derive(Clone, Copy)]
pub struct ImageTransformCall<'raw> {
    pub family: ImageFamily,
    pub raw: &'raw [u8],
    pub optional_params: &'raw IndexMap<String, Json>,
    pub request_data: &'raw IndexMap<String, Json>,
    pub created_seconds: i64,
}

// the family transforms parse the reply, log it, then convert it and set the family reply defaults
pub fn transform_image_reply(
    call: ImageTransformCall<'_>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<ImageGenerationReply, ImageTransformError> {
    let original: Json = serde_json::from_slice(call.raw)?;
    post_call(
        state,
        &PostCall {
            input: call
                .request_data
                .get("prompt")
                .cloned()
                .unwrap_or_else(|| Json::Text(String::new())),
            original_response: original,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(call.request_data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    );
    let mut reply = decode_image_reply(call.raw, call.created_seconds)?;
    apply_family_defaults(call.family, &mut reply, call.optional_params)?;
    Ok(reply)
}
