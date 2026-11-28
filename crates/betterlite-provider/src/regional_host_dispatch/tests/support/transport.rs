use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::gigachat_files::GigachatFileTransport;
use http::HeaderMap;

pub(crate) struct FakeTransport {
    uploads: Arc<AtomicUsize>,
}

impl FakeTransport {
    pub(crate) fn new() -> Self {
        Self {
            uploads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.uploads)
    }
}

impl GigachatFileTransport for FakeTransport {
    fn upload(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.uploads.fetch_add(1, Ordering::SeqCst);
        ready(Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: br#"{"id":"file-1"}"#.to_vec(),
        }))
    }
}
