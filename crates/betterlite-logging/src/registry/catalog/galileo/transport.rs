use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio::runtime::Handle;
use url::Url;
use url::form_urlencoded;

use crate::galileo::delivery::headers::Header;
use crate::galileo::delivery::transport::{HttpResponse, Transport, TransportError};

// LiteLLM's shared logging client falls back to this total, so a dead intake cannot wedge a flush forever
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

type Payload = IndexMap<String, Json>;
type Reply = Result<HttpResponse, TransportError>;

pub(super) struct PooledTransport {
    pub(super) client: ProxiedClient,
}

impl Transport for PooledTransport {
    fn get(&self, url: &str, headers: &[Header]) -> Reply {
        self.exchange(Method::GET, url, headers, RequestBody::Empty)
    }

    fn post_form(&self, url: &str, headers: &[Header], form: &[(&str, &str)]) -> Reply {
        let encoded = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form.iter().copied())
            .finish();
        let body = RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: encoded.into_bytes(),
        };
        self.exchange(Method::POST, url, headers, body)
    }

    fn post_json(&self, url: &str, headers: &[Header], payload: &Payload) -> Reply {
        let body = serde_json::to_vec(payload)
            .map_err(|error| transport_error(url, &error.to_string()))?;
        self.exchange(Method::POST, url, headers, RequestBody::Json(body))
    }
}

impl PooledTransport {
    // The merged trait is synchronous, so the request drives the pooled async client from the blocking thread the adapter spawns it on.
    fn exchange(
        &self,
        method: Method,
        url: &str,
        headers: &[Header],
        body: RequestBody,
    ) -> Result<HttpResponse, TransportError> {
        let parsed = Url::parse(url).map_err(|error| transport_error(url, &error.to_string()))?;
        let mut map = HeaderMap::new();
        for header in headers {
            let name = HeaderName::try_from(header.name.as_str())
                .map_err(|error| transport_error(url, &error.to_string()))?;
            let mut value = HeaderValue::try_from(header.value.expose_secret())
                .map_err(|error| transport_error(url, &error.to_string()))?;
            value.set_sensitive(true);
            map.insert(name, value);
        }
        let outbound = OutboundRequest {
            method,
            url: parsed,
            query: Vec::new(),
            headers: map,
            body,
            deadline: REQUEST_TIMEOUT,
        };
        let handle =
            Handle::try_current().map_err(|error| transport_error(url, &error.to_string()))?;
        let response = handle
            .block_on(self.client.send_buffered(outbound))
            .map_err(|error| transport_error(url, &error.to_string()))?;
        let text = String::from_utf8(response.body)
            .map_err(|error| transport_error(url, &error.to_string()))?;
        Ok(HttpResponse {
            status: response.status,
            body: text,
        })
    }
}

fn transport_error(url: &str, reason: &str) -> TransportError {
    TransportError {
        url: url.to_owned(),
        reason: reason.to_owned(),
    }
}
