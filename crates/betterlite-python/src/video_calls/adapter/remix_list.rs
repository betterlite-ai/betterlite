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

use super::OpenAiVideoConfig;
use crate::video_calls::convert::ConvertError;
use crate::video_calls::convert::fields::json;

mod list;
mod remix;

// what a request transform hands the transport, and what a reply turns into
type Rendered = Result<(String, Params), ConfigFailure>;
type VideoConverted = Result<VideoObject, ConfigFailure>;
type PageConverted = Result<Params, ConfigFailure>;

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
    ) -> Rendered {
        remix::transformed(video_id, prompt, extra_body, context)
    }

    fn transform_video_remix_response(&self, reply: VideoReply<'_>) -> VideoConverted {
        remix::converted(reply)
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Rendered {
        list::transformed(query, context)
    }

    fn transform_video_list_response(&self, reply: VideoReply<'_>) -> PageConverted {
        list::converted(reply)
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
