use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::Future;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use crate::api::responses_objects::ResponsesApiResponse;
use crate::error::{OpenAiError, ProviderError, ProviderErrorParts};

pub mod created_at;
pub mod event_decode;
mod tests;

pub const OPENAI_RESPONSES_API_BASE: &str = "https://api.openai.com/v1";
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = true;

pub struct RawResponse<'response> {
    pub status: u16,
    pub headers: &'response IndexMap<String, String>,
    pub text: &'response str,
}

pub enum ReplyBody {
    Typed(Box<ResponsesApiResponse>),
    Lax(IndexMap<String, Json>),
}

pub struct ResponsesReply {
    pub response: ReplyBody,
    pub headers: IndexMap<String, String>,
    pub additional_headers: AdditionalHeaders,
}

// LiteLLM's transform_response: the post-call log sees the sent text, then the reply is decoded
pub fn transform_response(
    reply: &RawResponse<'_>,
    now: i64,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<ResponsesReply, OpenAiError> {
    post_call(
        state,
        &PostCall {
            original_response: Json::Text(reply.text.to_owned()),
            input: Json::Null,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(IndexMap::new())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    );
    let parsed =
        serde_json::from_str::<Json>(reply.text).map_err(|_encode| provider_failure(reply))?;
    let Json::Object(mut fields) = parsed else {
        return Err(provider_failure(reply));
    };
    let created = fields
        .get("created_at")
        .ok_or_else(|| provider_failure(reply))?;
    let timestamp = created_at::convert_created_at(created, now)
        .map_err(|_conversion| provider_failure(reply))?;
    fields.insert(
        String::from("created_at"),
        Json::Number(Number::from(timestamp)),
    );
    let response = decode_reply(&fields)?;
    Ok(ResponsesReply {
        response,
        headers: reply.headers.clone(),
        additional_headers: process_response_headers(HeaderSource::Upstream(reply.headers)),
    })
}

fn decode_reply(fields: &IndexMap<String, Json>) -> Result<ReplyBody, OpenAiError> {
    let value = serde_json::to_value(Json::Object(fields.clone()))
        .map_err(|source| OpenAiError::Encode { source })?;
    match serde_json::from_value::<ResponsesApiResponse>(value) {
        Ok(response) => Ok(ReplyBody::Typed(Box::new(response))),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the Responses reply did not validate; keeping its fields as sent"
            );
            Ok(ReplyBody::Lax(fields.clone()))
        }
    }
}

fn provider_failure(reply: &RawResponse<'_>) -> OpenAiError {
    OpenAiError::Provider(Box::new(ProviderError::from(ProviderErrorParts {
        status_code: reply.status,
        message: reply.text.to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ApiKeySources<'sources> {
    pub litellm_params: Option<&'sources SecretString>,
    pub litellm: Option<&'sources SecretString>,
    pub openai: Option<&'sources SecretString>,
    pub environment: Option<&'sources SecretString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesProvider {
    OpenAi,
    Other,
}

pub trait WorkloadIdentity {
    fn bearer_token(
        &mut self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Option<SecretString>;
}

// LiteLLM's validate_environment, with the B501 identity resolution reached through the injected seam
#[must_use]
pub fn validate_environment<Identity>(
    mut headers: BTreeMap<String, String>,
    sources: ApiKeySources<'_>,
    api_base: Option<&str>,
    provider: ResponsesProvider,
    identity: &mut Identity,
) -> BTreeMap<String, String>
where
    Identity: WorkloadIdentity,
{
    let api_key = resolved_api_key(sources);
    headers
        .entry(String::from("Content-Type"))
        .or_insert_with(|| String::from("application/json"));
    let token = if matches!(provider, ResponsesProvider::OpenAi) {
        identity.bearer_token(api_key, api_base)
    } else {
        None
    };
    let offered = match (token, api_key) {
        (Some(workload), _key) => workload.expose_secret().to_owned(),
        (None, Some(key)) => key.expose_secret().to_owned(),
        (None, None) => String::from("None"),
    };
    headers.insert(String::from("Authorization"), format!("Bearer {offered}"));
    headers
}

fn resolved_api_key(sources: ApiKeySources<'_>) -> Option<&SecretString> {
    [
        sources.litellm_params,
        sources.litellm,
        sources.openai,
        sources.environment,
    ]
    .into_iter()
    .flatten()
    .find(|key| !key.expose_secret().is_empty())
}

// LiteLLM's get_complete_url: the first nonempty base, its trailing slashes trimmed, then /responses
#[must_use]
pub fn get_complete_url(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    env_base_url: Option<&str>,
    env_api_base: Option<&str>,
) -> String {
    let base = [api_base, litellm_api_base, env_base_url, env_api_base]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or(OPENAI_RESPONSES_API_BASE);
    format!("{}/responses", base.trim_end_matches('/'))
}

pub trait NativeStreamingSupport {
    type Error: Display;

    fn supports_native_streaming(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Result<Json, Self::Error>> + Send;
}

// LiteLLM's should_fake_stream: only an exact false from the lower implementation asks for fake streaming
pub async fn should_fake_stream<Capabilities>(
    model: Option<&str>,
    stream: Option<bool>,
    custom_llm_provider: Option<&str>,
    capabilities: &mut Capabilities,
) -> bool
where
    Capabilities: NativeStreamingSupport,
{
    if stream != Some(true) {
        return false;
    }
    let Some(named_model) = model else {
        return false;
    };
    let request = CapabilityRequest {
        model: named_model,
        custom_llm_provider,
    };
    match capabilities.supports_native_streaming(&request).await {
        Ok(Json::Bool(false)) => true,
        Ok(
            Json::Null
            | Json::Bool(true)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => false,
        Err(_failure) => {
            tracing::debug!(
                model = model,
                "the native streaming lookup failed; no fake stream"
            );
            false
        }
    }
}
