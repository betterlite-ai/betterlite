use std::sync::Arc;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use http::HeaderMap;
use tokio_util::task::TaskTracker;

use super::async_lifecycle::AsyncPassthroughStream;
use super::error::{PassthroughStreamError, UpstreamStatus};
use super::logging::SpendLogging;
use super::response::{AsyncResponse, PendingRelay, StreamBody};
use super::spend_collection::{OwnedProviderSpend, SpendSink, provider_spend};

mod tests;

pub struct PassthroughSend<'call> {
    pub client: &'call dyn PassthroughSender,
    pub request: OutboundRequest,
    pub is_streaming_request: bool,
}

// LiteLLM's client.client.send: the answer is a coroutine the relay awaits later, so the client outlives this borrow
pub trait PassthroughSender: Send + Sync {
    fn send_passthrough(&self, request: OutboundRequest) -> PendingSend;
}

// client.client.send answers with a coroutine on an async client and a plain response on a sync one
pub enum PendingSend {
    Coroutine(PendingRelay),
    Immediate,
}

pub struct FullReply {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub enum AsyncPassthroughReply<S, L>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
{
    Full(Box<FullReply>),
    Stream(Box<AsyncPassthroughStream<S, L>>),
}

impl PassthroughSender for ProxiedClient {
    fn send_passthrough(&self, request: OutboundRequest) -> PendingSend {
        let client = self.clone();
        let body_limit = self.limits.body_bytes;
        PendingSend::Coroutine(Box::pin(async move {
            let mut response = client
                .send_following(request)
                .await
                .map_err(|error| PassthroughStreamError::Send(Box::new(error)))?;
            let status_code = response.response.status().as_u16();
            let headers = std::mem::take(response.response.headers_mut());
            Ok(AsyncResponse {
                status_code,
                headers,
                source: Box::new(StreamBody {
                    chunks: response.body_stream(),
                    body_limit,
                }),
            })
        }))
    }
}

// LiteLLM's _async_passthrough_request, the one async entry point every pass-through hop goes through
pub async fn async_passthrough_request<C, L>(
    call: PassthroughSend<'_>,
    provider_config: C,
    logging: Arc<L>,
    background_tasks: TaskTracker,
) -> Result<AsyncPassthroughReply<OwnedProviderSpend<C>, L>, PassthroughStreamError>
where
    C: BasePassthroughConfig + Sync + Send + 'static,
    C::Logged: Send + 'static,
    L: SpendLogging<C::Logged>,
{
    let pending = match call.client.send_passthrough(call.request) {
        PendingSend::Coroutine(pending) => pending,
        PendingSend::Immediate => return Err(PassthroughStreamError::ExpectedCoroutine),
    };
    if !call.is_streaming_request {
        let mut response = pending.await?;
        let body = response
            .source
            .read_body()
            .await
            .map_err(|error| PassthroughStreamError::BodyRead(Box::new(error)))?;
        if !(200..300).contains(&response.status_code) {
            return Err(PassthroughStreamError::UpstreamStatus(Box::new(
                UpstreamStatus {
                    status_code: response.status_code,
                    headers: response.headers,
                    body,
                },
            )));
        }
        return Ok(AsyncPassthroughReply::Full(Box::new(FullReply {
            status_code: response.status_code,
            headers: response.headers,
            body,
        })));
    }
    let spend = provider_spend(provider_config, logging.spend_target());
    let mut stream = AsyncPassthroughStream::new(pending, spend, logging, background_tasks);
    stream.init().await?;
    Ok(AsyncPassthroughReply::Stream(Box::new(stream)))
}
