use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_wire::header_map;
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
use secrecy::SecretString;
use url::Url;

use super::credentials::{huggingface_chat_key, huggingface_env, huggingface_hub_token};
use super::hub::{self, PrefetchedHub};
use super::json;
use super::{ChatContext, DispatchDeps, HubDispatchError};
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;

async fn mapping_source(
    model: &str,
    api_base: Option<&str>,
    deps: &DispatchDeps<'_>,
) -> Result<Option<PrefetchedHub>, HubDispatchError> {
    if api_base.is_some_and(|base| !base.is_empty()) {
        return Ok(None);
    }
    let Some((_, rest)) = model.split_once('/') else {
        return Ok(None);
    };
    if !rest.contains('/') {
        return Ok(None);
    }
    let token = huggingface_hub_token(deps.environment);
    Ok(Some(PrefetchedHub(
        hub::fetch_mapping(deps.hub, rest, token.as_ref()).await?,
    )))
}

async fn request_body(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<ChatObject, HubDispatchError> {
    let params: ChatObject = json::convert(call.optional_params)?;
    let messages: Vec<ChatMessage> = call
        .messages
        .iter()
        .map(|message| json::convert(message))
        .collect::<Result<_, _>>()?;
    let body = match mapping_source(call.model, call.api_base, deps).await? {
        Some(prefetched) => {
            let token = huggingface_hub_token(deps.environment);
            let source = ProviderMappingCache::new(token, prefetched);
            transform_request(call.model, &messages, &params, call.api_base, &source)?
        }
        None => transform_request(call.model, &messages, &params, call.api_base, &NoMapping)?,
    };
    Ok(body)
}

fn prepared(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    key: Option<&SecretString>,
    body: &ChatObject,
) -> Result<PreparedRequest, HubDispatchError> {
    let environment = huggingface_env(deps.environment);
    let headers = call
        .headers
        .or(deps.globals.headers)
        .cloned()
        .unwrap_or_default();
    let url = get_complete_url(call.api_base, call.model, &environment)?;
    let resolved = validate_environment(&headers, key);
    let prepared = PreparedChat {
        http: PreparedHttp {
            method: Method::POST,
            url: Url::parse(&url).map_err(|source| HubDispatchError::Url {
                url: url.clone(),
                source,
            })?,
            headers: header_map(&resolved)
                .map_err(|source| HubDispatchError::Headers { source })?,
            body: serde_json::to_vec(body).map_err(|source| HubDispatchError::Encode { source })?,
            deadline: deps.timeout,
            body_limit: None,
        },
        retry: TransformRetry::DISABLED,
        client_model: Some(call.model.to_owned()),
    };
    if matches!(call.optional_params.get("stream"), Some(Json::Bool(true))) {
        return Ok(PreparedRequest::Stream(Box::new(prepared)));
    }
    Ok(PreparedRequest::Chat(Box::new(prepared)))
}

pub async fn chat_request(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<PreparedRequest, HubDispatchError> {
    let key = huggingface_chat_key(call.api_key, deps.globals, deps.environment);
    let body = request_body(call, deps).await?;
    prepared(call, deps, key.as_ref(), &body)
}

struct NoMapping;

impl ProviderMappingSource for NoMapping {
    fn mapping(&self, _model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        Err(HuggingfaceChatError::MappingMissing)
    }
}
