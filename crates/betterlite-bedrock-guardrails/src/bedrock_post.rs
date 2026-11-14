use std::future::Future;
use std::str::FromStr as _;
use std::time::Duration;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{
    AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue,
};
use url::{ParseError, Url};

use crate::bedrock_transport::BedrockGuardrail;
use crate::guardrail_error_response::parse_error_response;

pub const DEFAULT_GUARDRAIL_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub enum PostError {
    #[error("the Bedrock guardrail request failed with status {status}: {detail}")]
    Http { status: u16, detail: String },
    #[error("the Bedrock guardrail request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the signed request URL cannot be parsed")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error("the signed request header {name} has an invalid name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the signed request header {name} has an invalid value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
}

pub trait GuardrailTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl GuardrailTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's AsyncHTTPHandler follows redirects and raises for every non-2xx reply
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

pub trait TransportLog {
    fn guardrail_failed_to_respond(&self, detail: &str);
}

pub async fn sign_and_post<T: GuardrailTransport, L: TransportLog>(
    guardrail: &BedrockGuardrail,
    prepared_request: &AwsSignedRequest,
    transport: &T,
    log: &L,
    log_transport_failure: bool,
) -> Result<UpstreamResponse, PostError> {
    let request = outbound_request(
        prepared_request,
        guardrail.timeout.unwrap_or(DEFAULT_GUARDRAIL_TIMEOUT),
    )?;
    let response = match transport.send(request).await {
        Ok(response) => response,
        Err(source) => {
            let detail = source.to_string();
            tracing::error!("Bedrock AI: failed to make guardrail request: {detail}");
            log.guardrail_failed_to_respond(&detail);
            return Err(PostError::Transport { source });
        }
    };
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    let (status, detail) = parse_error_response(response.status, &response.body);
    if log_transport_failure {
        log.guardrail_failed_to_respond(&detail);
    }
    Err(PostError::Http { status, detail })
}

fn outbound_request(
    prepared_request: &AwsSignedRequest,
    timeout: Duration,
) -> Result<OutboundRequest, PostError> {
    let url = Url::parse(&prepared_request.url).map_err(|source| PostError::Url { source })?;
    let mut headers = HeaderMap::with_capacity(prepared_request.headers.len());
    for (name, value) in &prepared_request.headers {
        let header_name = HeaderName::from_str(name).map_err(|source| PostError::HeaderName {
            name: name.clone(),
            source,
        })?;
        let mut header_value =
            HeaderValue::from_str(value).map_err(|source| PostError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        headers.insert(header_name, header_value);
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(prepared_request.body.clone()),
        deadline: timeout,
    })
}
