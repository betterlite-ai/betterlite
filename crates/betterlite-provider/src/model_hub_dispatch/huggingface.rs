use betterlite_openai::chat::ChatMessage;
use betterlite_openai::huggingface_chat::error::HuggingfaceChatError;
use betterlite_openai::huggingface_chat::headers::validate_environment;
use betterlite_openai::huggingface_chat::mapping::{
    ProviderMapping, ProviderMappingCache, ProviderMappingSource,
};
use betterlite_openai::huggingface_chat::request::transform_request;
use betterlite_openai::huggingface_chat::url::get_complete_url;
use betterlite_openai::openai_base_config::Object as ChatObject;
use betterlite_prompts::arguments::json::{Json, Object};
use http::Method;
use secrecy::{ExposeSecret, SecretString};

use super::credentials::{
    HUGGINGFACE_CHAT_KEY, HUGGINGFACE_HUB_TOKEN, Sources, huggingface_env, resolve,
};
use super::hub::{EXPAND, PrefetchedHub, mapping_headers, model_info};
use super::json;
use super::transport::WireRequest;
use super::{ChatContext, DispatchDeps, HubDispatchError};
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;

async fn mapping_source(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    token: Option<&SecretString>,
) -> Result<Option<PrefetchedHub>, HubDispatchError> {
    if call.api_base.is_some_and(|base| !base.is_empty()) {
        return Ok(None);
    }
    let Some((_, rest)) = call.model.split_once('/') else {
        return Ok(None);
    };
    if !rest.contains('/') {
        return Ok(None);
    }
    let info = model_info(deps, rest, &mapping_headers(token), &[EXPAND]).await?;
    Ok(Some(PrefetchedHub(info)))
}

async fn request_body(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<ChatObject, HubDispatchError> {
    let params = json::convert::<Object, ChatObject>(call.optional_params)?;
    let messages: Vec<ChatMessage> = call
        .messages
        .iter()
        .map(json::convert)
        .collect::<Result<_, _>>()?;
    let sources = Sources {
        call: None,
        globals: deps.globals,
        env: deps.environment,
    };
    let token = resolve(HUGGINGFACE_HUB_TOKEN, &sources, &mut Object::new())?;
    let body = match mapping_source(call, deps, token.as_ref()).await? {
        Some(prefetched) => {
            let source = ProviderMappingCache::new(token, prefetched);
            transform_request(call.model, &messages, &params, call.api_base, &source)?
        }
        None => transform_request(call.model, &messages, &params, call.api_base, &NoMapping)?,
    };
    Ok(body)
}

// the call's headers, else litellm.headers, under the chat key chain and the resolved URL
fn chat_http(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    body: &ChatObject,
) -> Result<PreparedHttp, HubDispatchError> {
    let keys = Sources {
        call: call.api_key.map(ExposeSecret::expose_secret),
        globals: deps.globals,
        env: deps.environment,
    };
    let key = resolve(HUGGINGFACE_CHAT_KEY, &keys, &mut Object::new())?;
    let headers = call
        .headers
        .or(deps.globals.headers)
        .cloned()
        .unwrap_or_default();
    let url = get_complete_url(
        call.api_base,
        call.model,
        &huggingface_env(deps.environment),
    )?;
    let bytes = serde_json::to_vec(body).map_err(|source| HubDispatchError::Encode { source })?;
    let wire = WireRequest {
        method: Method::POST,
        url: &url,
        query: &[],
        headers: &validate_environment(&headers, key.as_ref()),
        body: None,
        deadline: deps.timeout,
    };
    let outbound = wire.outbound()?;
    Ok(PreparedHttp {
        method: outbound.method,
        url: outbound.url,
        headers: outbound.headers,
        body: bytes,
        deadline: outbound.deadline,
        body_limit: None,
    })
}

fn prepared(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    body: &ChatObject,
) -> Result<PreparedRequest, HubDispatchError> {
    let chat = PreparedChat {
        http: chat_http(call, deps, body)?,
        retry: TransformRetry::DISABLED,
        client_model: Some(call.model.to_owned()),
    };
    if matches!(call.optional_params.get("stream"), Some(Json::Bool(true))) {
        return Ok(PreparedRequest::Stream(Box::new(chat)));
    }
    Ok(PreparedRequest::Chat(Box::new(chat)))
}

pub async fn chat_request(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<PreparedRequest, HubDispatchError> {
    let body = request_body(call, deps).await?;
    prepared(call, deps, &body)
}

pub struct NoMapping;

impl ProviderMappingSource for NoMapping {
    fn mapping(&self, _model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        Err(HuggingfaceChatError::MappingMissing)
    }
}
