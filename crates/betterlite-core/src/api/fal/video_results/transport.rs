use std::future::Future;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::api::fal::video_prepare::VideoHeaders;

use super::error::VideoResultsError;

// both retrieval steps issue one plain GET, so the sync and async entry points differ only in which transport they pick
pub trait VideoTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl VideoTransport for ProxiedClient {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.limits.read_timeout,
        };
        // LiteLLM's provider client follows redirects, so a CDN answer for the artifact is fetched the same way
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

pub struct SuppliedTransport<'client, T> {
    pub kind: ClientKind,
    pub transport: &'client T,
}

pub struct VideoClients<'clients, T> {
    pub sync: &'clients T,
    pub asynchronous: &'clients T,
}

impl<T> VideoClients<'_, T> {
    #[must_use]
    pub const fn for_kind(&self, kind: ClientKind) -> &T {
        match kind {
            ClientKind::Sync => self.sync,
            ClientKind::Asynchronous => self.asynchronous,
        }
    }
}

pub fn header_map(headers: &VideoHeaders) -> Result<HeaderMap, VideoResultsError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            VideoResultsError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| VideoResultsError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.append(header_name, header_value);
    }
    Ok(map)
}
