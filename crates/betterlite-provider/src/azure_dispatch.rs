use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use serde::Serialize;

use crate::operation::{
    ChatOperation, EmbeddingOperation, OperationError, OperationRequest, OperationResult,
    RequestKind,
};
use crate::prepared::body::chat_request_fields;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::{TransformOnUnprocessable, send_with_retry};

pub mod call;
pub mod compose;
pub mod dispatch;
pub mod error;
mod resolve;
mod wire;

mod tests;

// ChatDeployment falls back to this base for a deployment that names none; Azure reads it as an unset api_base
const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com/v1";

// The call kwargs LiteLLM reads before the provider body; none of them is sent to Azure
const DISCARDED_KEYS: [&str; 9] = [
    "api_type",
    "drop_params",
    "allowed_openai_params",
    "additional_drop_params",
    "stream_timeout",
    "num_retries",
    "organization",
    "mock_delay",
    "mock_timeout",
];

const EMBEDDING_KEYS: [&str; 10] = [
    "api_base",
    "api_key",
    "api_version",
    "timeout",
    "max_retries",
    "headers",
    "extra_headers",
    "extra_body",
    "azure_ad_token",
    "metadata",
];

// B010's stubs, filled by this slice: the provider marker and its typed records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Azure;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzurePrepared;

// The typed dispatcher's answer: the requests it composed, the reply it projected, and the failure it met
#[derive(Debug, Clone, Serialize)]
pub struct AzureResult {
    pub requests: Vec<dispatch::RecordedRequest>,
    pub reply: Json,
    pub error: Json,
}

impl Adapter for Azure {
    const NAME: &'static str = "azure";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        let composition = call::Composition::process();
        match request.kind() {
            RequestKind::Chat(chat) => {
                compose::prepare_chat(&composition, chat_call(chat, deployment, false)?)
            }
            RequestKind::Stream(chat) => {
                compose::prepare_chat(&composition, chat_call(chat, deployment, true)?)
            }
            RequestKind::Embedding(embedding) => {
                compose::prepare_embedding(&composition, embedding_call(embedding, deployment)?)
            }
            RequestKind::Other => Err(unsupported()),
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
        dispatch::decode(prepared, upstream)
    }
}

impl TransformOnUnprocessable for Azure {}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(Azure::NAME),
        operation: String::from("provider endpoint"),
    }
}

fn chat_call(
    chat: &ChatOperation,
    deployment: &Deployment,
    stream: bool,
) -> Result<call::ChatCall, OperationError> {
    let fields = chat_request_fields(&chat.request)?;
    let messages: Vec<Message> = round_trip(&fields.messages)?;
    let mut optional: Object = round_trip(&fields.params)?;
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
    let mut request: betterlite_openai::embedding::EmbeddingRequest = embedding.request.clone();
    let mut optional = Object::new();
    for name in EMBEDDING_KEYS {
        if let Some(value) = request.passthrough.shift_remove(name) {
            let parsed: Json = serde_json::from_value(value).map_err(encode)?;
            optional.insert(name.to_owned(), parsed);
        }
    }
    let credentials = credentials(&mut optional, deployment)?;
    Ok(call::EmbeddingCall {
        model: embedding.model.clone(),
        request,
        credentials,
    })
}

fn round_trip<T>(value: &impl Serialize) -> Result<T, OperationError>
where
    T: serde::de::DeserializeOwned,
{
    let text = serde_json::to_string(value).map_err(|source| OperationError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| OperationError::Encode { source })
}

// ChatDeployment falls back to the OpenAI base for a deployment that names none.
fn deployment_base(deployment: &Deployment) -> Option<String> {
    if deployment.api_base == OPENAI_DEFAULT_BASE {
        return None;
    }
    Some(deployment.api_base.clone())
}

fn take_text(optional: &mut Object, name: &str) -> Option<String> {
    match optional.shift_remove(name) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn take_number(optional: &mut Object, name: &str) -> Option<f64> {
    match optional.shift_remove(name) {
        Some(Json::Number(number)) => number.as_f64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn take_headers(
    optional: &mut Object,
    name: &str,
) -> Result<Option<Headers>, error::AzureProviderError> {
    let Some(value) = optional.shift_remove(name) else {
        return Ok(None);
    };
    let Json::Object(fields) = value else {
        return Err(error::AzureProviderError::HeadersNotObject);
    };
    let mut headers = Headers::new();
    for (header, value) in fields {
        let Json::Text(text) = value else {
            return Err(error::AzureProviderError::HeaderValueType { name: header });
        };
        headers.insert(header, text);
    }
    Ok(Some(headers))
}

fn credentials(
    optional: &mut Object,
    deployment: &Deployment,
) -> Result<call::Credentials, OperationError> {
    for name in DISCARDED_KEYS {
        optional.shift_remove(name);
    }
    let api_base = take_text(optional, "api_base").or_else(|| deployment_base(deployment));
    let api_version = take_text(optional, "api_version");
    let api_key = take_text(optional, "api_key")
        .map(SecretString::from)
        .or_else(|| deployment.api_key.clone());
    let headers = take_headers(optional, "headers")?;
    let extra_headers = take_headers(optional, "extra_headers")?;
    let max_retries = optional.shift_remove("max_retries");
    let extra_body = optional.shift_remove("extra_body");
    let azure_ad_token = take_text(optional, "azure_ad_token").map(SecretString::from);
    let timeout =
        take_number(optional, "timeout").or_else(|| Some(deployment.timeout.as_secs_f64()));
    Ok(call::Credentials {
        api_base,
        api_version,
        api_key,
        headers,
        extra_headers,
        max_retries,
        extra_body,
        azure_ad_token,
        azure_ad_token_provider: None,
        base_model: None,
        timeout,
        cloudflare_params: betterlite_azure::endpoints::cloudflare::CloudflareParams::default(),
    })
}
