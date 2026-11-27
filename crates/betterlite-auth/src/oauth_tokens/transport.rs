use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use thiserror::Error;
use url::Url;

use crate::oauth_tokens::request::FORM_CONTENT_TYPE;

// get_async_httpx_client's default total timeout, from LiteLLM's COMPLETION_HTTP_FALLBACK_SECONDS
pub const TOKEN_INFO_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct TokenInfoRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub form: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct TokenInfoResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TokenInfoTransportError {
    #[error("the token info request failed: {message}")]
    Failed { message: String },
}

#[async_trait]
pub trait TokenInfoTransport: Send + Sync {
    async fn send(
        &self,
        request: TokenInfoRequest,
    ) -> Result<TokenInfoResponse, TokenInfoTransportError>;
}

#[async_trait]
impl TokenInfoTransport for ProxiedClient {
    async fn send(
        &self,
        request: TokenInfoRequest,
    ) -> Result<TokenInfoResponse, TokenInfoTransportError> {
        // an introspection POST carries its parameters as a form body; a token info GET carries none
        let body = if request.method == Method::POST {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            for (name, value) in &request.form {
                form.append_pair(name, value);
            }
            RequestBody::Bytes {
                content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
                data: form.finish().into_bytes(),
            }
        } else {
            RequestBody::Empty
        };
        let response = self
            .send_buffered(OutboundRequest {
                method: request.method,
                url: request.url,
                query: Vec::new(),
                headers: request.headers,
                body,
                deadline: TOKEN_INFO_TIMEOUT,
            })
            .await
            .map_err(|error| TokenInfoTransportError::Failed {
                message: error.to_string(),
            })?;
        Ok(TokenInfoResponse {
            status: response.status,
            body: response.body,
        })
    }
}
