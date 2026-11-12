use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::{HeaderMap, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;
use url::{Host, Url};

use crate::compatible_wire::header_map;
use crate::openai_base_config::Headers;

pub const DEFAULT_MODEL_LIST_BASE: &str = "https://api.openai.com";
const MODEL_LIST_PATH: &str = "/v1/models";
const CHAT_COMPLETIONS: &str = "chat/completions";

// get_complete_url: a substring check, not URL query-aware joining
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .unwrap_or(DEFAULT_MODEL_LIST_BASE)
        .trim_end_matches('/');
    if base.contains(CHAT_COMPLETIONS) {
        return base.to_owned();
    }
    format!("{base}/{CHAT_COMPLETIONS}")
}

// validate_environment: an api_key overwrites Authorization and a missing content type is filled in
#[must_use]
pub fn validate_environment(api_key: Option<&SecretString>, headers: &Headers) -> Headers {
    let mut result = headers.clone();
    if let Some(key) = api_key {
        result.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    if !result.contains_key("content-type") && !result.contains_key("Content-Type") {
        result.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
    }
    result
}

#[derive(Debug, Error)]
pub enum ModelListError {
    #[error("api_base is not a valid URL")]
    InvalidBase {
        #[source]
        source: url::ParseError,
    },
    #[error("api_base names no host")]
    MissingHost,
    #[error("the request headers are invalid")]
    Headers {
        #[source]
        source: http::Error,
    },
    #[error("the model list request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the model list request answered status {status}")]
    Status { status: u16 },
    #[error("the model list reply is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the model list reply has no data list")]
    MissingData,
    #[error("a model list entry has no id")]
    MissingModelId,
}

pub trait ModelListTransport: Send + Sync {
    fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl ModelListTransport for ProxiedClient {
    async fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> Result<UpstreamResponse, TransportError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: url.clone(),
            query: Vec::new(),
            headers: headers.clone(),
            body: RequestBody::Empty,
            deadline,
        };
        self.send_buffered(request).await
    }
}

pub struct ModelListRequest<'base> {
    pub api_key: Option<SecretString>,
    pub api_base: Option<&'base str>,
    pub environment_api_key: Option<SecretString>,
    pub deadline: Duration,
}

pub async fn get_models<T>(
    transport: &T,
    request: &ModelListRequest<'_>,
) -> Result<Vec<String>, ModelListError>
where
    T: ModelListTransport,
{
    let url = models_url(request.api_base)?;
    let key = request
        .api_key
        .as_ref()
        .or(request.environment_api_key.as_ref());
    let authorization = key.map_or_else(
        || String::from("Bearer None"),
        |resolved| format!("Bearer {}", resolved.expose_secret()),
    );
    let headers = header_map(&Headers::from([(
        String::from("Authorization"),
        authorization,
    )]))
    .map_err(|source| ModelListError::Headers { source })?;
    let body = fetch(transport, &url, &headers, request.deadline).await?;
    model_ids(&body)
}

async fn fetch<T>(
    transport: &T,
    url: &Url,
    headers: &HeaderMap,
    deadline: Duration,
) -> Result<Vec<u8>, ModelListError>
where
    T: ModelListTransport,
{
    let response = transport
        .get(url, headers, deadline)
        .await
        .map_err(|source| ModelListError::Transport { source })?;
    if response.status != 200 {
        return Err(ModelListError::Status {
            status: response.status,
        });
    }
    Ok(response.body)
}

fn model_ids(body: &[u8]) -> Result<Vec<String>, ModelListError> {
    let reply: Value =
        serde_json::from_slice(body).map_err(|source| ModelListError::Decode { source })?;
    let models = reply
        .get("data")
        .and_then(Value::as_array)
        .ok_or(ModelListError::MissingData)?;
    models
        .iter()
        .map(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(ModelListError::MissingModelId)
        })
        .collect()
}

// get_models strips api_base to scheme, host and any explicit port before it appends /v1/models
fn models_url(api_base: Option<&str>) -> Result<Url, ModelListError> {
    let base = api_base.unwrap_or(DEFAULT_MODEL_LIST_BASE);
    let parsed = Url::parse(base).map_err(|source| ModelListError::InvalidBase { source })?;
    let host = match parsed.host() {
        Some(Host::Domain(name)) => name.to_owned(),
        Some(Host::Ipv4(address)) => address.to_string(),
        Some(Host::Ipv6(address)) => address.to_string(),
        None => return Err(ModelListError::MissingHost),
    };
    let scheme = parsed.scheme();
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let url = format!("{scheme}://{host}{port}{MODEL_LIST_PATH}");
    Url::parse(&url).map_err(|source| ModelListError::InvalidBase { source })
}
