use betterlite_core::api::video_remix_list_transport::error::ConfigFailure;
use betterlite_core::api::video_remix_list_transport::records::{
    Params, VideoListQuery, VideoReply, VideoRequestContext,
};
use betterlite_openai::videos::list::{list_request, list_response};

use super::super::support::config_failure;
use crate::video_calls::convert::fields::json;
use betterlite_openai::videos::VideoOptions;

// the url and body the list transform hands the transport, from the call's own query
pub(super) fn transformed(
    query: VideoListQuery<'_>,
    context: VideoRequestContext<'_>,
) -> Result<(String, Params), ConfigFailure> {
    let extra = query
        .extra_query
        .map_or_else(|| Ok(VideoOptions::new()), json)
        .map_err(config_failure)?;
    let (url, data) = list_request(
        context.api_base,
        query.after,
        query.limit,
        query.order,
        &extra,
    );
    Ok((url, json(&data).map_err(config_failure)?))
}

// the page a list reply carries
pub(super) fn converted(reply: &VideoReply<'_>) -> Result<Params, ConfigFailure> {
    let list = list_response(&reply.raw_response.body, reply.custom_llm_provider)
        .map_err(config_failure)?;
    json(&list).map_err(config_failure)
}
