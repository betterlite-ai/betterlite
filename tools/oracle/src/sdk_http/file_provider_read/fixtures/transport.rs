use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::{
    ClientSource, WriteRequest, WriteTransport,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderName, HeaderValue};

// The pooled client the metadata handler would create; the case's mock answers every request it sends
pub(crate) struct DispatchSource {
    pub(crate) client: ProxiedClient,
}

impl ClientSource for DispatchSource {
    type Client = WireClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<Self::Client, ClientError> {
        Ok(WireClient {
            client: self.client.clone(),
            headers: BTreeMap::new(),
        })
    }
}

// A supplied HTTPHandler's own httpx client sends every request with its headers, so the wire carries them
pub(crate) struct WireClient {
    pub(crate) client: ProxiedClient,
    pub(crate) headers: BTreeMap<String, String>,
}

impl WriteTransport for WireClient {
    fn default_timeout(&self) -> Duration {
        WriteTransport::default_timeout(&self.client)
    }

    async fn send(&self, mut request: WriteRequest) -> Result<UpstreamResponse, TransportError> {
        let lent = self.headers.iter().filter_map(|(name, value)| {
            let header_name = HeaderName::from_bytes(name.as_bytes()).ok()?;
            let header_value = HeaderValue::from_str(value).ok()?;
            Some((header_name, header_value))
        });
        request.headers.extend(lent);
        WriteTransport::send(&self.client, request).await
    }
}
