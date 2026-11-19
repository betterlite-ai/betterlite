use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use http::header::CONTENT_TYPE;
use indexmap::IndexMap;

use super::client::SelectedClient;

// B516 builds the SDK's wire request; the Azure client only adds its auth header and query
pub(super) struct Patched<'client, T> {
    pub(super) client: &'client SelectedClient,
    pub(super) inner: &'client T,
    pub(super) extra_headers: Option<&'client IndexMap<String, String>>,
}

impl<T: FileTransport> FileTransport for Patched<'_, T> {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.inner
            .send_buffered(self.client.sign(request, self.extra_headers))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.inner
            .send_stream(self.client.sign(request, self.extra_headers))
    }
}

// B516's delete decode never sees the reply headers, and the SDK picks JSON parsing from the content type
pub(super) struct Recording<'client, T> {
    pub(super) inner: Patched<'client, T>,
    pub(super) content_type: &'client Mutex<Option<String>>,
}

impl<T: FileTransport> FileTransport for Recording<'_, T> {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let content_type = self.content_type;
        let send = self.inner.send_buffered(request);
        async move {
            let reply = send.await?;
            let observed = reply
                .headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            *content_type.lock().unwrap_or_else(PoisonError::into_inner) = observed;
            Ok(reply)
        }
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.inner.send_stream(request)
    }
}
