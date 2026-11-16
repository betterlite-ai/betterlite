use std::future::Future;
use std::time::Duration;

use betterlite_anthropic::count_tokens::error::CountTokensError;
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_anthropic::count_tokens::validate::validate_request;
use betterlite_anthropic::count_tokens::wire::{CountTokensBody, transform_request};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use url::Url;

use crate::anthropic_count::endpoint::get_count_tokens_endpoint;
use crate::anthropic_count::error::ApiError;
use crate::anthropic_count::headers::{HeadersRequest, get_required_headers};
use crate::anthropic_count::reply::{FoundryCountReply, decode_foundry_count_reply};
use crate::anthropic_messages::environment::{Headers, ParamsKey, TokenSource};

const SENSITIVE_HEADERS: [&str; 3] = ["x-api-key", "api-key", "authorization"];

pub struct CountTokensRequest<'call> {
    pub body: CountTokensBody<'call>,
    pub api_key: &'call SecretString,
    pub api_base: &'call str,
    pub params_key: ParamsKey<'call>,
    pub token: &'call TokenSource<'call>,
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct FoundryCountHandler<T> {
    pub transport: T,
    pub request_timeout: Duration,
    pub beta_version: String,
    pub configured_key: Option<SecretString>,
}

impl<T: CountTokensTransport> FoundryCountHandler<T> {
    // The request borrows credential sources that are not Sync, so the future only owns what was built before it starts
    pub fn handle_count_tokens_request<'handler>(
        &'handler self,
        request: &CountTokensRequest<'_>,
    ) -> impl Future<Output = Result<FoundryCountReply, ApiError>> + Send + use<'handler, T> {
        let prepared = self.outbound_request(request);
        async move {
            let response = self
                .transport
                .send(prepared?)
                .await
                .map_err(|source| CountTokensError::Transport { source })?;
            if response.status != 200 {
                let status_code = response.status;
                let text = String::from_utf8(response.body).unwrap_or_else(|error| {
                    format!("{} bytes that are not UTF-8", error.as_bytes().len())
                });
                return Err(ApiError::Upstream { status_code, text });
            }
            Ok(decode_foundry_count_reply(&response.body)?)
        }
    }

    fn outbound_request(
        &self,
        request: &CountTokensRequest<'_>,
    ) -> Result<OutboundRequest, ApiError> {
        // LiteLLM's Foundry handler validates the model and messages only, without system or tools
        validate_request(&CountTokensBody {
            model: request.body.model,
            messages: request.body.messages,
            tools: None,
            system: None,
            optional_params: request.body.optional_params,
        })?;
        let body = transform_request(&request.body);
        let endpoint = get_count_tokens_endpoint(request.api_base);
        let url = Url::parse(&endpoint).map_err(|source| CountTokensError::Endpoint { source })?;
        let auth = HeadersRequest {
            api_key: request.api_key,
            params_key: request.params_key,
            configured_key: self.configured_key.as_ref(),
            token: request.token,
        };
        let headers = request_header_map(&get_required_headers(&auth, &self.beta_version)?)?;
        Ok(OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(
                serde_json::to_vec(&body).map_err(|source| CountTokensError::Encode { source })?,
            ),
            deadline: request.timeout.unwrap_or(self.request_timeout),
        })
    }
}

// LiteLLM's httpx client refuses a header name or value it cannot carry, before sending anything
fn header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), CountTokensError> {
    let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_invalid| {
        CountTokensError::InvalidHeaderName {
            name: name.to_owned(),
        }
    })?;
    let mut header_value =
        HeaderValue::from_str(value).map_err(|_invalid| CountTokensError::InvalidHeaderValue {
            name: name.to_owned(),
        })?;
    header_value.set_sensitive(SENSITIVE_HEADERS.contains(&header_name.as_str()));
    Ok((header_name, header_value))
}

fn request_header_map(headers: &Headers) -> Result<HeaderMap, CountTokensError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let (header_name, header_value) = header_pair(name, value)?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}
