use std::error::Error as StdError;
use std::future::Future;
use std::string::FromUtf8Error;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

pub const API_KEY_HEADER: &str = "x-argilla-api-key";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgillaMethod {
    Get,
    Post,
    Put,
}

pub struct ArgillaRequest {
    pub method: ArgillaMethod,
    pub url: String,
    pub api_key: SecretString,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

pub struct ArgillaResponse {
    pub status: u16,
    pub body: String,
}

// A non-2xx answer is an Ok response; only a request that never got an answer is an Err.
pub trait Transport: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn send(
        &self,
        request: ArgillaRequest,
    ) -> impl Future<Output = Result<ArgillaResponse, Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("the Argilla URL is not valid")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the Argilla API key cannot be a header value")]
    ApiKeyHeader {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the Argilla response is not UTF-8")]
    Body {
        #[source]
        source: FromUtf8Error,
    },
}

pub struct PooledTransport {
    pub client: ProxiedClient,
}

impl Transport for PooledTransport {
    type Error = HttpError;

    async fn send(&self, request: ArgillaRequest) -> Result<ArgillaResponse, Self::Error> {
        let ArgillaRequest {
            method,
            url,
            api_key,
            body,
            timeout,
        } = request;
        let parsed = match Url::parse(&url) {
            Ok(parsed) => parsed,
            Err(source) => return Err(HttpError::Url { source }),
        };
        let mut value = HeaderValue::try_from(api_key.expose_secret())
            .map_err(|source| HttpError::ApiKeyHeader { source })?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static(API_KEY_HEADER), value);
        let outbound = OutboundRequest {
            method: match method {
                ArgillaMethod::Get => Method::GET,
                ArgillaMethod::Post => Method::POST,
                ArgillaMethod::Put => Method::PUT,
            },
            url: parsed,
            query: Vec::new(),
            headers,
            body: body.map_or(RequestBody::Empty, RequestBody::Json),
            deadline: timeout,
        };
        let response = self.client.send_buffered(outbound).await?;
        Ok(ArgillaResponse {
            status: response.status,
            body: String::from_utf8(response.body).map_err(|source| HttpError::Body { source })?,
        })
    }
}
