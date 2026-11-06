use std::fmt;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::Url;

use crate::openai_base_config::Headers;

pub struct Post<'post> {
    pub url: &'post str,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl fmt::Debug for Post<'_> {
    // a header value can carry an access token, so the derived form would print it into any log line
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Post")
            .field("url", &self.url)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransportFailure {
    #[error("the GitHub request URL is not valid")]
    Url(#[source] url::ParseError),
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the GitHub request failed")]
    Request(#[source] TransportError),
    #[error("the GitHub transport is unavailable")]
    Unavailable,
}

pub trait Transport {
    fn post_json<'call>(
        &'call mut self,
        post: Post<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call;
}

fn convert_header(
    name: &str,
    value: String,
) -> Result<(HeaderName, HeaderValue), TransportFailure> {
    let header_name =
        HeaderName::try_from(name).map_err(|source| TransportFailure::HeaderName {
            name: name.to_owned(),
            source,
        })?;
    let header_value =
        HeaderValue::try_from(value).map_err(|source| TransportFailure::HeaderValue {
            name: name.to_owned(),
            source,
        })?;
    Ok((header_name, header_value))
}

#[derive(Debug, Clone)]
pub struct HttpTransport {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

impl Transport for HttpTransport {
    async fn post_json(&mut self, post: Post<'_>) -> Result<Reply, TransportFailure> {
        let url = Url::parse(post.url).map_err(TransportFailure::Url)?;
        let mut headers = HeaderMap::new();
        for (name, value) in post.headers {
            let (header_name, header_value) = convert_header(&name, value)?;
            headers.insert(header_name, header_value);
        }
        let request = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(post.body),
            deadline: self.deadline,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(TransportFailure::Request)?;
        Ok(Reply {
            status: response.status,
            body: response.body,
        })
    }
}
