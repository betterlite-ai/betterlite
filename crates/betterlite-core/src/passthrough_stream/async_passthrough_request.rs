use std::sync::Arc;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::async_lifecycle::AsyncPassthroughStream;
use super::error::{PassthroughStreamError, UpstreamStatus};
use super::logging::SpendLogging;
use super::response::{AsyncResponse, PendingRelay, StreamBody};
use super::spend_collection::{OwnedProviderSpend, SpendCollection, SpendSink};

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
        return Ok(AsyncPassthroughReply::Full(Box::new(
            full_reply(pending).await?,
        )));
    }
    let target = logging.spend_target();
    let mut stream = AsyncPassthroughStream {
        initialized: false,
        status_code: 0,
        headers: HeaderMap::new(),
        pending: Some(pending),
        source: None,
        spend: SpendCollection {
            sink: Some(OwnedProviderSpend {
                config: provider_config,
                model: target.model.to_owned(),
                custom_llm_provider: target.custom_llm_provider.to_owned(),
                endpoint: target.endpoint.to_owned(),
                chunks: Vec::new(),
                collected_bytes: 0,
                disabled: false,
            }),
            chunk_count: 0,
            failed: false,
        },
        logging,
        background_tasks,
        flush_scheduled: false,
        hidden_params: IndexMap::new(),
    };
    stream.init().await?;
    Ok(AsyncPassthroughReply::Stream(Box::new(stream)))
}

async fn full_reply(pending: PendingRelay) -> Result<FullReply, PassthroughStreamError> {
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
    Ok(FullReply {
        status_code: response.status_code,
        headers: response.headers,
        body,
    })
}
