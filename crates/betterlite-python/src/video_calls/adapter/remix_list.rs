use std::collections::BTreeMap;

use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_remix_list_transport::error::{ConfigFailure, ErrorHeaders};
use betterlite_core::api::video_remix_list_transport::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoReply, VideoRequestContext,
};
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::complete_url;
use betterlite_openai::videos::error::get_error_class;
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};

use super::OpenAiVideoConfig;
use super::support::config_failure;
use crate::video_calls::convert::ConvertError;
use crate::video_calls::convert::fields::json;

// LiteLLM's OpenAIVideoConfig over the remix and list transport: the four transforms and the error class
impl VideoRemixListConfig for OpenAiVideoConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.headers(request.headers, request.api_key, request.litellm_params)
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, ConfigFailure> {
        Ok(complete_url(api_base))
    }

    fn transform_video_remix_request(
        &self,
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

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure> {
        let video = remix_response(&reply.raw_response.body, reply.custom_llm_provider)
            .map_err(config_failure)?;
        json(&video).map_err(config_failure)
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let extra = options(query.extra_query).map_err(config_failure)?;
        let (url, data) = list_request(
            context.api_base,
            query.after,
            query.limit,
            query.order,
            &extra,
        );
        Ok((url, json(&data).map_err(config_failure)?))
    }

    fn transform_video_list_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<Params, ConfigFailure> {
        let list = list_response(&reply.raw_response.body, reply.custom_llm_provider)
            .map_err(config_failure)?;
        json(&list).map_err(config_failure)
    }

    // get_error_class: the headers' text values, through B525's own error class
    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ErrorHeaders,
    ) -> ProviderError {
        get_error_class(message, status_code, text_headers(headers))
    }
}

// the params a handler holds as this crate's JSON, as the options B525's transforms read
fn options(params: Option<&Params>) -> Result<VideoOptions, ConvertError> {
    match params {
        Some(entries) => json(entries),
        None => Ok(VideoOptions::new()),
    }
}

// an error header is a JSON value; the ones a reply carries as text are the ones a ProviderError keeps
fn text_headers(headers: ErrorHeaders) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name, text)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}
