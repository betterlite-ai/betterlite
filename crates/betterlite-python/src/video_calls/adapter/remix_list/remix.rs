use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_list_transport::error::ConfigFailure;
use betterlite_core::api::video_remix_list_transport::records::{
    Params, VideoReply, VideoRequestContext,
};
use betterlite_openai::videos::remix::{remix_request, remix_response};

use super::super::support::config_failure;
use super::options;
use crate::video_calls::convert::fields::json;

// the url and body the remix transform hands the transport, from the call's own record
pub(super) fn transformed(
    video_id: &str,
    prompt: &str,
    extra_body: Option<&Params>,
    context: VideoRequestContext<'_>,
) -> Result<(String, Params), ConfigFailure> {
    let extra = options(extra_body).map_err(config_failure)?;
    let (url, data) =
        remix_request(video_id, prompt, context.api_base, &extra).map_err(config_failure)?;
    Ok((url, json(&data).map_err(config_failure)?))
}

// the object a remix reply carries
pub(super) fn converted(reply: VideoReply<'_>) -> Result<VideoObject, ConfigFailure> {
    let video = remix_response(&reply.raw_response.body, reply.custom_llm_provider)
        .map_err(config_failure)?;
    json(&video).map_err(config_failure)
}
