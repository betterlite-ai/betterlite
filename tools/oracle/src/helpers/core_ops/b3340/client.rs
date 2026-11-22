use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_core::api::video_remix_list_transport::client::{
    VideoAsyncClient, VideoClientSource, VideoRequest, VideoSyncClient,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::identity::provider_id::ProviderId;
use http::HeaderMap;

// The client's own read timeout: a request that carries it is one the caller named no timeout for
pub(crate) const CLIENT_TIMEOUT_SECONDS: f64 = 123.0;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(123);

// One client that records every request and answers the same reply
#[derive(Clone)]
pub(crate) struct Wire {
    status: u16,
    body: Vec<u8>,
    requests: Arc<Mutex<Vec<VideoRequest>>>,
}

impl Wire {
    pub(crate) fn replying(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        })
    }
}

impl VideoSyncClient for Wire {
    fn send(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        self.answer(request)
    }
}

impl VideoAsyncClient for Wire {
    fn default_timeout(&self) -> Duration {
        CLIENT_TIMEOUT
    }

    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(request))
    }
}

// The pool: one wire, so every handler's request lands in the same list
#[derive(Clone)]
pub(crate) struct Source {
    wire: Wire,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self { wire }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<VideoRequest>> {
        self.wire.requests()
    }
}

impl VideoClientSource for Source {
    type SyncClient = Wire;
    type AsyncClient = Wire;
    type Error = io::Error;

    fn sync_client(&self, _ssl_verify: Option<bool>) -> Result<Wire, io::Error> {
        Ok(self.wire.clone())
    }

    fn async_client(
        &self,
        _provider: ProviderId,
        _ssl_verify: Option<bool>,
    ) -> Result<Wire, io::Error> {
        Ok(self.wire.clone())
    }
}
