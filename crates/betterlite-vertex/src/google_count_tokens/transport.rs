use std::collections::BTreeMap;
use std::{future::Future, time::Duration};

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, transport_error::TransportError};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::google_count_tokens::cleaner::clean_contents;
use crate::google_count_tokens::decode::{GoogleCountResponse, decode_google_count};
use crate::google_count_tokens::error::GoogleCountError;

#[derive(serde::Serialize)]
struct CountBody<'contents> {
    contents: &'contents Json,
}

pub struct CountRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

// B1316's pooled Gemini client in production and a scripted reply in tests
pub trait CountTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl CountTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx handlers follow redirects, so every hop is re-checked here too
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

// LiteLLM sends exactly {contents: cleaned_contents}; tools and system never join the count body
pub fn count_body(contents: &Json) -> Result<Vec<u8>, GoogleCountError> {
    let cleaned = clean_contents(contents)?;
    serde_json::to_vec(&CountBody { contents: &cleaned }).map_err(|source| GoogleCountError::Json {
        path: "the count request body".to_owned(),
        source,
    })
}

pub fn header_map(headers: &BTreeMap<String, SecretString>) -> Result<HeaderMap, GoogleCountError> {
    let mut map = HeaderMap::new();
    for (name, secret) in headers {
        let header =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| GoogleCountError::Header {
                source: http::Error::from(source),
            })?;
        let mut value = HeaderValue::from_str(secret.expose_secret()).map_err(|source| {
            GoogleCountError::Header {
                source: http::Error::from(source),
            }
        })?;
        value.set_sensitive(true);
        map.insert(header, value);
    }
    Ok(map)
}

pub async fn count<T: CountTransport>(
    transport: &T,
    request: CountRequest,
    deadline: Duration,
) -> Result<GoogleCountResponse, GoogleCountError> {
    let url = Url::parse(&request.url).map_err(|source| GoogleCountError::Url { source })?;
    let outbound = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: request.headers,
        body: RequestBody::Json(request.body),
        deadline,
    };
    let reply = transport
        .send(outbound)
        .await
        .map_err(|source| GoogleCountError::Connection { source })?;
    if reply.status >= 400 {
        let body = str::from_utf8(&reply.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", reply.body.len()),
            str::to_owned,
        );
        return Err(GoogleCountError::Status {
            status: reply.status,
            body,
        });
    }
    decode_google_count(&reply.body)
}
