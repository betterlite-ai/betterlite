use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::config::VideoConfigLookup;
use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_remix_list_transport::error::{ConfigFailure, ErrorHeaders};
use betterlite_core::api::video_remix_list_transport::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoReply, VideoRequestContext,
};
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{VideoCredentials, complete_url, validate_environment};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::VideoObject as OpenAiVideoObject;
use secrecy::{ExposeSecret as _, SecretString};

// The OpenAI video config over B525's free functions: the adapter the call layer hands the transport
pub(crate) struct OpenAiConfig;

fn failed<E>(error: E) -> ConfigFailure
where
    E: StdError + Send + Sync + 'static,
{
    ConfigFailure {
        source: Box::new(error),
    }
}

fn options(params: &Params) -> Result<VideoOptions, ConfigFailure> {
    let encoded = serde_json::to_value(params).map_err(failed)?;
    serde_json::from_value(encoded).map_err(failed)
}

fn params_from(options: &VideoOptions) -> Result<Params, ConfigFailure> {
    let encoded = serde_json::to_value(options).map_err(failed)?;
    serde_json::from_value(encoded).map_err(failed)
}

fn core_video(video: &OpenAiVideoObject) -> Result<VideoObject, ConfigFailure> {
    let encoded = serde_json::to_value(video).map_err(failed)?;
    serde_json::from_value(encoded).map_err(failed)
}

fn params_key(params: Option<&Params>) -> Option<SecretString> {
    match params?.get("api_key")? {
        Json::Text(text) => Some(SecretString::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn input_headers(headers: &Headers) -> BTreeMap<String, SecretString> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

impl VideoRemixListConfig for OpenAiConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        let key = match request.api_key {
            Some(key) => Some(key.clone()),
            None => params_key(request.litellm_params),
        };
        let credentials = VideoCredentials {
            api_key: key,
            ..VideoCredentials::default()
        };
        validate_environment(input_headers(request.headers), &credentials)
            .into_iter()
            .map(|(name, value)| (name, value.expose_secret().to_owned()))
            .collect()
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
        let extra = match extra_body {
            Some(params) => options(params)?,
            None => VideoOptions::new(),
        };
        let (url, body) =
            remix_request(video_id, prompt, context.api_base, &extra).map_err(failed)?;
        Ok((url, params_from(&body)?))
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure> {
        let video =
            remix_response(&reply.raw_response.body, reply.custom_llm_provider).map_err(failed)?;
        core_video(&video)
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let extra = match query.extra_query {
            Some(params) => options(params)?,
            None => VideoOptions::new(),
        };
        let (url, params) = list_request(
            context.api_base,
            query.after,
            query.limit,
            query.order,
            &extra,
        );
        Ok((url, params_from(&params)?))
    }

    fn transform_video_list_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<Params, ConfigFailure> {
        let list =
            list_response(&reply.raw_response.body, reply.custom_llm_provider).map_err(failed)?;
        let encoded = serde_json::to_value(&list).map_err(failed)?;
        serde_json::from_value(encoded).map_err(failed)
    }

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: ErrorHeaders,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

// The lookup the call layer is given: only the OpenAI tag resolves
pub(crate) struct Lookup {
    openai: OpenAiConfig,
}

impl Lookup {
    pub(crate) fn new() -> Self {
        Self {
            openai: OpenAiConfig,
        }
    }
}

impl VideoConfigLookup for Lookup {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn VideoRemixListConfig> {
        match tag {
            VideoConfigTag::OpenAi => Some(&self.openai),
            VideoConfigTag::Azure
            | VideoConfigTag::Gemini
            | VideoConfigTag::VertexAi
            | VideoConfigTag::RunwayMl
            | VideoConfigTag::FalAi
            | VideoConfigTag::HostedVllm(_) => None,
        }
    }
}
