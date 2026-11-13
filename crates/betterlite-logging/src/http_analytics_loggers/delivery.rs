use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use http::{HeaderMap, Method};
use url::Url;

// LiteLLM's module level client falls back to 600 seconds, so an analytics POST waits no longer than that
pub const DELIVERY_DEADLINE: Duration = Duration::from_secs(600);

pub struct PostRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

impl fmt::Debug for PostRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // a PromptLayer body carries its api key in a JSON field, so Debug never prints body bytes
        f.debug_struct("PostRequest")
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body", &format_args!("[{} bytes]", self.body.len()))
            .field("deadline", &self.deadline)
            .finish()
    }
}

#[derive(Debug)]
pub struct PostResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait Delivery: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError<E>
where
    E: StdError + 'static,
{
    #[error("the {name} URL could not be parsed")]
    Url {
        name: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header could not be built")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("{name} is not set")]
    MissingSetting { name: &'static str },
    #[error("the payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the {name} response is not JSON")]
    Response {
        name: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the service answered with status {status}")]
    Status { status: u16 },
    #[error("the delivery failed")]
    Transport(#[source] E),
}

#[derive(Clone, Debug)]
pub struct PooledDelivery {
    pub client: ProxiedClient,
}

impl Delivery for PooledDelivery {
    type Error = TransportError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let outbound = OutboundRequest {
            method: Method::POST,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Json(request.body),
            deadline: request.deadline,
        };
        async move {
            let response = self.client.send_buffered(outbound).await?;
            Ok(PostResponse {
                status: response.status,
                body: response.body,
            })
        }
    }
}
