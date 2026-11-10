use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use url::Url;

use crate::count_tokens::error::{ApiError, CountTokensError};
use crate::count_tokens::headers::{DEFAULT_COUNT_TOKENS_ENDPOINT, get_required_headers};
use crate::count_tokens::validate::validate_request;
use crate::count_tokens::wire::{
    CountResponse, CountTokensRequest, decode_count_response, transform_request,
};

pub trait CountTokensTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl CountTokensTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_buffered(request).await
    }
}

#[derive(Debug)]
pub struct AnthropicCountTokensHandler<T> {
    pub transport: T,
    pub request_timeout: Duration,
    pub beta_version: String,
}

impl<T: CountTokensTransport> AnthropicCountTokensHandler<T> {
    // LiteLLM re-raises AnthropicError untouched and wraps every other exception in a 500 AnthropicError
    pub async fn handle_count_tokens_request(
        &self,
        request: &CountTokensRequest<'_>,
    ) -> Result<CountResponse, ApiError> {
        validate_request(&request.body)?;
        let body = transform_request(&request.body);
        let endpoint = request
            .api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(DEFAULT_COUNT_TOKENS_ENDPOINT);
        let url = Url::parse(endpoint).map_err(|source| CountTokensError::Endpoint { source })?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: get_required_headers(request.api_key, &self.beta_version)?,
            body: RequestBody::Json(
                serde_json::to_vec(&body).map_err(|source| CountTokensError::Encode { source })?,
            ),
            deadline: request.timeout.unwrap_or(self.request_timeout),
        };
        tracing::debug!(url = %outbound.url, "sending the Anthropic CountTokens request");
        let response = self
            .transport
            .send(outbound)
            .await
            .map_err(|source| CountTokensError::Transport { source })?;
        tracing::debug!(
            status = response.status,
            "the Anthropic CountTokens endpoint answered"
        );
        if response.status != 200 {
            let status = response.status;
            let message = response_text(response.body);
            tracing::error!("the Anthropic CountTokens endpoint answered {status}: {message}");
            return Err(ApiError::Upstream {
                status_code: status,
                text: message,
            });
        }
        Ok(decode_count_response(&response.body)?)
    }
}

// LiteLLM reads response.text, which requires text; betterlite refuses bytes that are not UTF-8 by name
fn response_text(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not UTF-8", error.as_bytes().len()))
}
