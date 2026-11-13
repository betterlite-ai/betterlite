use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{CustomProxyInput, TargetProvider, check_custom_proxy};
use crate::model_urls::model_name::get_vertex_base_model_name;
use crate::model_urls::{ApiVersion, Endpoint};
use crate::token_cache::probe::AccessToken;

use super::error::GemmaPlanError;

pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const BEARER_PREFIX: &str = "Bearer ";
pub const PREDICT_SUFFIX: &str = ":predict";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invocation {
    Sync,
    Async,
}

// the four Python client types: HTTPHandler and httpx.Client serve the sync call, their async twins the async one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppliedClientKind {
    SyncHandler,
    SyncRaw,
    AsyncHandler,
    AsyncRaw,
}

// the isinstance branch B1336's post adapter needs, since a raw client leaves the timeout out when it is None
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallerStyle {
    Handler,
    Raw,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: SuppliedClientKind,
    pub client: &'client Client,
}

pub struct GemmaCall<'text> {
    pub model: &'text str,
    pub messages: &'text [ChatMessage],
    pub optional_params: &'text Object,
    pub api_base: Option<&'text str>,
    pub credentials: &'text AccessToken,
    pub stream: bool,
    pub invocation: Invocation,
    pub timeout: Option<Duration>,
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

#[derive(Debug)]
pub struct GemmaPlan {
    pub model: String,
    pub url: String,
    pub stream: bool,
    pub headers: BTreeMap<String, SecretString>,
    pub caller: Option<CallerStyle>,
}

pub fn for_call(
    call: &GemmaCall<'_>,
    supplied: Option<SuppliedClientKind>,
) -> Result<GemmaPlan, GemmaPlanError> {
    // a Gemma deployment is its own endpoint, so project and location can never stand in for a base
    let api_base = call
        .api_base
        .filter(|base| !base.is_empty())
        .ok_or(GemmaPlanError::MissingApiBase)?;
    let mut headers = BTreeMap::new();
    headers.insert(
        AUTHORIZATION_HEADER.to_owned(),
        SecretString::from(format!(
            "{BEARER_PREFIX}{}",
            call.credentials.token.expose_secret()
        )),
    );
    headers.insert(
        CONTENT_TYPE_HEADER.to_owned(),
        SecretString::from(JSON_CONTENT_TYPE),
    );
    Ok(GemmaPlan {
        model: get_vertex_base_model_name(call.model).to_owned(),
        url: predict_url(api_base, call.stream)?,
        stream: call.stream,
        headers,
        caller: caller_style(call.invocation, supplied),
    })
}

fn predict_url(api_base: &str, stream: bool) -> Result<String, GemmaPlanError> {
    if api_base.ends_with(PREDICT_SUFFIX) {
        return Ok(api_base.to_owned());
    }
    let target = check_custom_proxy(&CustomProxyInput {
        api_base: Some(api_base),
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint: Endpoint::Predict.as_str(),
        stream,
        auth_header: None,
        url: api_base,
        model: None,
        project: None,
        location: None,
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

fn caller_style(
    invocation: Invocation,
    supplied: Option<SuppliedClientKind>,
) -> Option<CallerStyle> {
    match (invocation, supplied?) {
        (Invocation::Sync, SuppliedClientKind::SyncHandler)
        | (Invocation::Async, SuppliedClientKind::AsyncHandler) => Some(CallerStyle::Handler),
        (Invocation::Sync, SuppliedClientKind::SyncRaw)
        | (Invocation::Async, SuppliedClientKind::AsyncRaw) => Some(CallerStyle::Raw),
        (Invocation::Sync, SuppliedClientKind::AsyncHandler | SuppliedClientKind::AsyncRaw)
        | (Invocation::Async, SuppliedClientKind::SyncHandler | SuppliedClientKind::SyncRaw) => {
            None
        }
    }
}
