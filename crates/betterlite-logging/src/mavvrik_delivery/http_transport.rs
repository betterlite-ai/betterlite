use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use secrecy::ExposeSecret as _;
use url::{ParseError, Url};

use super::transport::{Method, Transport, TransportRequest, TransportResponse};

const API_KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");
// Mavvrik and GCS answer with a small JSON object or an error page, and only the first 400 bytes of a page are kept
pub const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct HttpTransport {
    pub client: ProxiedClient,
    pub body_limit: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpTransportError {
    #[error("the request URL is not valid")]
    Url(#[source] ParseError),
    #[error("the header name {name} is not valid")]
    HeaderName {
        name: &'static str,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header holds a character a header cannot carry")]
    HeaderValue {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Mavvrik API key holds a character a header cannot carry")]
    ApiKey(#[source] InvalidHeaderValue),
    #[error("the request could not be completed")]
    Send(#[source] TransportError),
}

const fn wire_method(method: Method) -> http::Method {
    match method {
        Method::Delete => http::Method::DELETE,
        Method::Get => http::Method::GET,
        Method::Patch => http::Method::PATCH,
        Method::Post => http::Method::POST,
        Method::Put => http::Method::PUT,
    }
}

fn outbound(request: TransportRequest) -> Result<OutboundRequest, HttpTransportError> {
    let url = Url::parse(&request.url).map_err(HttpTransportError::Url)?;
    let mut headers = HeaderMap::new();
    for (name, value) in request.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| HttpTransportError::HeaderName { name, source })?;
        let header_value = HeaderValue::try_from(value)
            .map_err(|source| HttpTransportError::HeaderValue { name, source })?;
        headers.insert(header_name, header_value);
    }
    if let Some(api_key) = request.api_key {
        let key =
            HeaderValue::try_from(api_key.expose_secret()).map_err(HttpTransportError::ApiKey)?;
        headers.insert(API_KEY_HEADER, key);
    }
    let body = if request.body.is_empty() {
        RequestBody::Empty
    } else {
        RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/octet-stream"),
            data: request.body,
        }
    };
    Ok(OutboundRequest {
        method: wire_method(request.method),
        url,
        query: Vec::new(),
        headers,
        body,
        deadline: request.timeout,
    })
}

impl Transport for HttpTransport {
    type Error = HttpTransportError;

    async fn send(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse, HttpTransportError> {
        let response = self
            .client
            .send(outbound(request)?)
            .await
            .map_err(HttpTransportError::Send)?
            .buffered(self.body_limit)
            .await
            .map_err(HttpTransportError::Send)?;
        // A header value that is not visible ASCII cannot be a Location, so it is left out
        let headers = response
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        Ok(TransportResponse {
            status: response.status,
            headers,
            body: response.body,
        })
    }
}
