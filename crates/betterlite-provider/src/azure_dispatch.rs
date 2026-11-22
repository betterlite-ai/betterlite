use std::fmt;

use crate::operation::{
    ChatOperation, EmbeddingOperation, OperationError, OperationRequest, OperationResult,
    RequestKind,
};
use crate::prepared::body::chat_request_fields;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::{TransformOnUnprocessable, send_with_retry};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use serde::Deserialize;
pub mod call;
pub mod compose;
pub mod error;
pub mod fixtures;
// operation.rs names the dispatch outcome, and the fixture module holds the type
pub(crate) use fixtures::types::AzureResult;
mod reply;
mod resolve;
mod tests;
mod wire;
// ChatDeployment falls back to this base for a deployment that names none; Azure reads it as an unset api_base
const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com/v1";
// The call kwargs LiteLLM reads before the provider body; none of them is sent to Azure
const DISCARDED_KEYS: &str = "api_type drop_params allowed_openai_params additional_drop_params stream_timeout num_retries organization mock_delay mock_timeout";
const EMBEDDING_KEYS: &str = "api_base api_key api_version timeout max_retries headers extra_headers extra_body azure_ad_token metadata";
const OPTION_KEYS: &str = "api_base api_version api_key headers extra_headers max_retries extra_body azure_ad_token timeout";
#[derive(Deserialize)]
struct CallOptions {
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<String>,
    headers: Option<Headers>,
    extra_headers: Option<Headers>,
    max_retries: Option<Json>,
    extra_body: Option<Json>,
    azure_ad_token: Option<String>,
    timeout: Option<f64>,
}
#[derive(Clone)]
pub struct Azure {
    pub(crate) composition: call::Composition,
}
// the composition is the environment the adapter reads, not its identity, and its values stay out of Debug output
impl fmt::Debug for Azure {
    #[cfg_attr(test, mutants::skip)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Azure").finish_non_exhaustive()
    }
}
impl PartialEq for Azure {
    #[cfg_attr(test, mutants::skip)]
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for Azure {}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureRequest;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzurePrepared;
impl Adapter for Azure {
    const NAME: &'static str = "azure";
    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        match request.kind() {
            RequestKind::Chat(chat) => prepared(
                &self.composition,
                call::Request::Chat(Box::new(chat_call(chat, deployment, false)?)),
            ),
            RequestKind::Stream(chat) => prepared(
                &self.composition,
                call::Request::Chat(Box::new(chat_call(chat, deployment, true)?)),
            ),
            RequestKind::Embedding(embedding) => prepared(
                &self.composition,
                call::Request::Embedding(Box::new(embedding_call(embedding, deployment)?)),
            ),
            RequestKind::Other => Err(reply::unsupported()),
        }
    }
    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        send_with_retry(client, prepared, deployment, self).await
    }
    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        reply::decode(prepared, upstream)
    }
}
impl TransformOnUnprocessable for Azure {}
fn prepared(
    composition: &call::Composition,
    request: call::Request,
) -> Result<PreparedRequest, OperationError> {
    compose::prepare(composition, request)
        .map(compose::prepared_request)
        .map_err(OperationError::from)
}
fn chat_call(
    chat: &ChatOperation,
    deployment: &Deployment,
    stream: bool,
) -> Result<call::ChatCall, OperationError> {
    let fields = chat_request_fields(&chat.request)?;
    let messages: Vec<Message> = resolve::convert(&fields.messages)?;
    let mut optional: Object = resolve::convert(&fields.params)?;
    let credentials = credentials(&mut optional, deployment)?;
    Ok(call::ChatCall {
        model: chat.model.clone(),
        messages,
        optional,
        credentials,
        stream,
        supplied: None,
    })
}
fn embedding_call(
    embedding: &EmbeddingOperation,
    deployment: &Deployment,
) -> Result<call::EmbeddingCall, OperationError> {
    let mut request: EmbeddingRequest = embedding.request.clone();
    let mut passthrough = Object::new();
    for name in EMBEDDING_KEYS.split(' ') {
        if let Some(value) = request.passthrough.shift_remove(name) {
            let parsed: Json = serde_json::from_value(value)
                .map_err(|source| OperationError::Encode { source })?;
            passthrough.insert(name.to_owned(), parsed);
        }
    }
    let credentials = credentials(&mut passthrough, deployment)?;
    Ok(call::EmbeddingCall {
        model: embedding.model.clone(),
        request,
        credentials,
    })
}
fn options(optional: &mut Object) -> Result<CallOptions, OperationError> {
    let mut values = Object::new();
    for name in OPTION_KEYS.split(' ') {
        if let Some(value) = optional.shift_remove(name) {
            values.insert(name.to_owned(), value);
        }
    }
    resolve::convert(&Json::Object(values)).map_err(OperationError::from)
}
fn credentials(
    optional: &mut Object,
    deployment: &Deployment,
) -> Result<call::Credentials, OperationError> {
    for name in DISCARDED_KEYS.split(' ') {
        optional.shift_remove(name);
    }
    let options = options(optional)?;
    Ok(call::Credentials {
        api_base: options.api_base.or_else(|| {
            (deployment.api_base != OPENAI_DEFAULT_BASE).then(|| deployment.api_base.clone())
        }),
        api_version: options.api_version,
        api_key: options
            .api_key
            .map(SecretString::from)
            .or_else(|| deployment.api_key.clone()),
        headers: options.headers,
        extra_headers: options.extra_headers,
        max_retries: options.max_retries,
        extra_body: options.extra_body,
        azure_ad_token: options.azure_ad_token.map(SecretString::from),
        base_model: None,
        timeout: options.timeout.or(Some(deployment.timeout.as_secs_f64())),
    })
}
