use std::future::Future;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
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
