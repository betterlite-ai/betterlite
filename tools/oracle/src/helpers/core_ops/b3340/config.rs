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
use serde::Serialize;
use serde::de::DeserializeOwned;

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

// B525's wire types and this helper's params are the same JSON, so each conversion is one round trip
fn decode<T, V>(value: &V) -> Result<T, ConfigFailure>
where
    T: DeserializeOwned,
    V: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(failed)?;
    let decoded: T = serde_json::from_value(encoded).map_err(failed)?;
    Ok(decoded)
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
        let key = request.api_key.map_or_else(
            || params_key(request.litellm_params),
            |key| Some(key.clone()),
        );
        let credentials = VideoCredentials {
            api_key: key,
            params_api_key: None,
            global_api_key: None,
            openai_key: None,
            environment_api_key: None,
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
        let (url, body) = remix_request(
            video_id,
            prompt,
            context.api_base,
            &extra_body.map_or(Ok(VideoOptions::new()), decode::<VideoOptions, Params>)?,
        )
        .map_err(failed)?;
        Ok((url, decode::<Params, _>(&body)?))
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure> {
        let body = &reply.raw_response.body;
        let video = remix_response(body, reply.custom_llm_provider).map_err(failed)?;
        decode::<VideoObject, _>(&video)
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let extra = query
            .extra_query
            .map_or(Ok(VideoOptions::new()), decode::<VideoOptions, Params>)?;
        let (url, params) = list_request(
            context.api_base,
            query.after,
            query.limit,
            query.order,
            &extra,
        );
        Ok((url, decode::<Params, _>(&params)?))
    }

    fn transform_video_list_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<Params, ConfigFailure> {
        let list = list_response(&reply.raw_response.body, reply.custom_llm_provider);
        list.map_err(failed)
            .and_then(|list| decode::<Params, _>(&list))
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
