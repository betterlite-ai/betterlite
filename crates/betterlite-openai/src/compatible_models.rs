use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::compatible_wire::header_map;
use crate::openai_base_config::{Headers, overlapping_api_base_path};
use error::ModelInfoError;

pub mod error;
mod tests;

pub const MODEL_INFO_REFRESH_SECONDS: u64 = 300;
pub const MODEL_INFO_REFRESH_CONCURRENCY: usize = 8;
pub const MODEL_INFO_DISCOVERY_PROVIDERS: [&str; 4] = [
    "hosted_vllm",
    "openai",
    "text-completion-openai",
    "openai_like",
];
const FAILURE_SECONDS: u64 = 60;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub trait ModelListCache: Send + Sync {
    fn get(&self, key: &str) -> Option<ModelList>;

    fn set(&self, key: &str, list: ModelList, ttl: Duration);
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ModelList {
    #[serde(default)]
    pub data: Vec<ModelCard>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ModelCard {
    pub id: String,
    pub max_model_len: Option<Value>,
    pub context_length: Option<Value>,
    pub max_input_tokens: Option<Value>,
    pub max_output_tokens: Option<Value>,
}

#[skip_serializing_none]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenLimits {
    pub max_tokens: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

pub struct ModelQuery<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: &'call Headers,
}

fn clamped(limit: Option<u64>, context: Option<u64>) -> Option<u64> {
    match (limit, context) {
        (Some(own), Some(cap)) => Some(own.min(cap)),
        (own, _) => own,
    }
}

impl ModelCard {
    // anything but a positive integer reads as absent, as a bool or a float does in LiteLLM
    #[must_use]
    pub fn token_limits(&self) -> TokenLimits {
        let [model_len, context_length, input, output] = [
            &self.max_model_len,
            &self.context_length,
            &self.max_input_tokens,
            &self.max_output_tokens,
        ]
        .map(|limit| {
            limit
                .as_ref()
                .and_then(Value::as_u64)
                .filter(|tokens| *tokens > 0)
        });
        let context = model_len.or(context_length);
        TokenLimits {
            max_tokens: context,
            max_input_tokens: clamped(input.or(context), context),
            max_output_tokens: clamped(output.or(context), context),
        }
    }
}

pub fn decode_compatible_model_list(bytes: &[u8]) -> Result<ModelList, ModelInfoError> {
    let value: Value = serde_json::from_slice(bytes).map_err(ModelInfoError::Decode)?;
    // pydantic reads one object whose cards are objects, while serde would also take a struct from a sequence
    if !is_model_object(&value) {
        return Err(ModelInfoError::NotAnObject);
    }
    serde_json::from_value(value).map_err(ModelInfoError::Decode)
}

fn is_model_object(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    match fields.get("data") {
        Some(Value::Array(cards)) => cards.iter().all(Value::is_object),
        Some(_) | None => true,
    }
}

#[must_use]
pub fn cache_key(url: &str, headers: &Headers) -> String {
    let items = headers
        .iter()
        .map(|(name, value)| Json::List(vec![Json::Text(name.clone()), Json::Text(value.clone())]))
        .collect();
    let identity = Json::List(vec![Json::Text(url.to_owned()), Json::List(items)]);
    let digest = Sha256::digest(python_json::dumps(&identity).as_bytes());
    format!("upstream_model_info:{}", hex::encode(digest))
}

pub async fn discover_model_limits(
    client: &ProxiedClient,
    cache: &dyn ModelListCache,
    query: &ModelQuery<'_>,
) -> Result<TokenLimits, ModelInfoError> {
    let url = overlapping_api_base_path(query.api_base, "/v1/models")
        .map_err(ModelInfoError::Endpoint)?;
    let key = cache_key(&url, query.headers);
    let list = if let Some(cached) = cache.get(&key) {
        cached
    } else {
        let (list, ttl) = match fetch_model_list(client, &url, query.headers).await {
            Ok(list) => (list, Duration::from_secs(MODEL_INFO_REFRESH_SECONDS)),
            Err(error) => {
                tracing::debug!(%error, "Could not discover upstream model token limits");
                (ModelList::default(), Duration::from_secs(FAILURE_SECONDS))
            }
        };
        cache.set(&key, list.clone(), ttl);
        list
    };
    Ok(list
        .data
        .iter()
        .find(|card| card.id == query.model)
        .map(ModelCard::token_limits)
        .unwrap_or_default())
}

async fn fetch_model_list(
    client: &ProxiedClient,
    url: &str,
    headers: &Headers,
) -> Result<ModelList, ModelInfoError> {
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(url).map_err(ModelInfoError::Url)?,
        query: Vec::new(),
        headers: header_map(headers).map_err(ModelInfoError::Header)?,
        body: RequestBody::Empty,
        deadline: DISCOVERY_TIMEOUT,
    };
    let response = client
        .send(request)
        .await
        .map_err(ModelInfoError::Transport)?
        .buffered(MAX_RESPONSE_BYTES)
        .await
        .map_err(ModelInfoError::Transport)?;
    if !(200..300).contains(&response.status) {
        return Err(ModelInfoError::Status {
            status: response.status,
        });
    }
    decode_compatible_model_list(&response.body)
}
