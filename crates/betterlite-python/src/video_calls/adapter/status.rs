use async_trait::async_trait;
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_status_transport::config::{VideoErrorClasses, VideoStatusConfig};
use betterlite_core::api::video_status_transport::error::{BaseLlmException, ProviderFailure};
use betterlite_core::api::video_status_transport::records::{
    Headers, Params, VideoEnvironment, VideoReply, VideoRequestContext,
};
use betterlite_openai::videos::config::complete_url;
use betterlite_openai::videos::segment::path_url;
use betterlite_openai::videos::status::status_response;
use http::header::{HeaderName, HeaderValue};

use super::OpenAiVideoConfig;
use super::support::{check_response, provider_failure};
use crate::video_calls::clients::StatusClient;
use crate::video_calls::convert::fields::json;

// LiteLLM's OpenAIVideoConfig over the status transport: the retrieve transform plus the error class
#[async_trait]
impl VideoStatusConfig for OpenAiVideoConfig {
    type SyncClient = StatusClient;
    type AsyncClient = StatusClient;

    fn validate_environment(
        &self,
        request: VideoEnvironment<'_>,
    ) -> Result<Headers, ProviderFailure> {
        for (name, value) in request.headers {
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| ProviderFailure {
                message: format!("invalid header name {name:?}: {source}"),
                status_code: Some(500),
                headers: None,
            })?;
            HeaderValue::from_str(value).map_err(|source| ProviderFailure {
                message: format!("invalid header value for {name:?}: {source}"),
                status_code: Some(500),
                headers: None,
            })?;
        }
        Ok(self.headers(request.headers, request.api_key, request.litellm_params))
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, ProviderFailure> {
        Ok(complete_url(api_base))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_request(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ProviderFailure> {
        let url = path_url(video_id, context.api_base, "").map_err(provider_failure)?;
        Ok((url, Params::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        _client: Option<&StatusClient>,
    ) -> Result<VideoObject, ProviderFailure> {
        check_response(reply.raw_response)?;
        let video = status_response(&reply.raw_response.body, reply.custom_llm_provider)
            .map_err(provider_failure)?;
        json(&video).map_err(provider_failure)
    }

    // ast-grep-ignore: no-long-names
    async fn async_transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&StatusClient>,
    ) -> Result<VideoObject, ProviderFailure> {
        self.transform_video_status_retrieve_response(reply, client)
    }
}

impl VideoErrorClasses for OpenAiVideoConfig {
    // get_error_class raises BaseLLMException with the facts the converter already read
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
            status_code_is_synthesized: false,
        }
    }
}
