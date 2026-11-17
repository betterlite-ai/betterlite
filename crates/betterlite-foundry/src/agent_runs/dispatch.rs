use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::agent_requests::prepare::{Headers, prepare};
use crate::agent_requests::request::AgentCallRequest;
use crate::agent_results::reply::AgentReply;
use crate::agent_results::request::{SSL_VERIFY_KEY, StreamRequest, build_stream_request};
use crate::credentials::AZURE_AI_AGENTS_SCOPE;
use crate::credentials::error::CredentialsError;

use super::error::AgentRunError;
use super::flow::CompletionCall;
use super::transport::AgentRunMode;

pub const AZURE_SCOPE_KEY: &str = "azure_scope";

// The config resolves the token through litellm.llms.azure.common_utils.get_azure_ad_token, which B529 owns
pub trait EntraResolver {
    fn resolve(
        &self,
        params: &Object,
        environment: &BTreeMap<String, String>,
    ) -> Result<Option<SecretString>, CredentialsError>;
}

pub struct DispatchCall<'data> {
    pub model: &'data str,
    pub messages: &'data [Object],
    pub optional_params: &'data Object,
    pub litellm_params: &'data Object,
    pub headers: &'data Headers,
    pub environment: &'data BTreeMap<String, String>,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub reply: AgentReply,
    pub acompletion: bool,
    pub stream: bool,
}

pub enum Route<'data> {
    Completion(CompletionCall<'data>),
    Stream(StreamRequest),
}

fn resolve_key(
    call: &DispatchCall<'_>,
    resolver: &dyn EntraResolver,
) -> Result<SecretString, AgentRunError> {
    if let Some(key) = call.api_key {
        return Ok(key.clone());
    }
    let mut params = call.litellm_params.clone();
    params.insert(
        String::from(AZURE_SCOPE_KEY),
        Json::Text(String::from(AZURE_AI_AGENTS_SCOPE)),
    );
    resolver
        .resolve(&params, call.environment)?
        .ok_or(AgentRunError::MissingToken)
}

pub fn dispatch<'data>(
    call: DispatchCall<'data>,
    resolver: &dyn EntraResolver,
) -> Result<Route<'data>, AgentRunError> {
    let api_key = resolve_key(&call, resolver)?;
    let request = AgentCallRequest {
        model: call.model,
        messages: call.messages,
        optional_params: call.optional_params,
        headers: call.headers,
        api_key: Some(&api_key),
        api_base: call.api_base,
    };
    let prepared = prepare(&request)?;
    if call.acompletion && call.stream {
        let stream = build_stream_request(
            &prepared,
            call.messages,
            call.optional_params,
            call.litellm_params,
        )?;
        return Ok(Route::Stream(stream));
    }
    Ok(Route::Completion(CompletionCall {
        model: call.model,
        messages: call.messages,
        optional_params: call.optional_params,
        prepared,
        ssl_verify: ssl_verify(call.litellm_params),
        reply: call.reply,
        mode: if call.acompletion {
            AgentRunMode::Async
        } else {
            AgentRunMode::Sync
        },
        supplied: None,
    }))
}

fn ssl_verify(litellm_params: &Object) -> Json {
    match litellm_params.get(SSL_VERIFY_KEY) {
        None | Some(Json::Null) => Json::Null,
        Some(value) => value.clone(),
    }
}
