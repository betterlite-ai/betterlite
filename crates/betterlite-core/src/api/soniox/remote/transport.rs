use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use tokio::time::sleep;

// Both traits require Sync so a request borrows its transport into a future any worker can drive
pub trait RemoteTransport: Sync {
    fn call(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl RemoteTransport for ProxiedClient {
    async fn call(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_buffered(request).await
    }
}

pub trait Sleeper: Sync {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        sleep(duration).await;
    }
}
