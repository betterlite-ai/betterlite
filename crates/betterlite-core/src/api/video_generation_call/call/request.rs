use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::VideoGenerationRequest;
use super::prepare::Prepared;
use crate::api::video_generation_call::handler::GenerationHandlerRequest;
use crate::api::video_generation_call::providers::VideoGenerationProviders;
use crate::api::videos_http_transport::client::{ClientKind, ClientSource, SuppliedClient};
use crate::api::videos_http_transport::video_config::Params;

pub(super) enum Polled<Output> {
    Ready(Output),
    StillWaiting,
}

pub(super) fn build_handler_request<'call, Providers>(
    request: VideoGenerationRequest,
    client: Option<&'call <Providers::Source as ClientSource>::Client>,
    is_async: bool,
    litellm_params: Params,
    prepared: Prepared<'call, Providers>,
) -> (
    GenerationHandlerRequest<'call, Providers::Source>,
    &'call Providers::Config,
    &'call Providers::Source,
    String,
    String,
)
where
    Providers: VideoGenerationProviders,
{
    let model = prepared.model.clone();
    let provider = prepared.provider.clone();
    let config = prepared.config;
    let source = prepared.source;
    let handler_request = handler_request(request, client, is_async, litellm_params, prepared);
    (handler_request, config, source, model, provider)
}

fn handler_request<'call, Providers>(
    request: VideoGenerationRequest,
    client: Option<&'call <Providers::Source as ClientSource>::Client>,
    is_async: bool,
    litellm_params: Params,
    prepared: Prepared<'call, Providers>,
) -> GenerationHandlerRequest<'call, Providers::Source>
where
    Providers: VideoGenerationProviders,
{
    GenerationHandlerRequest {
        model: prepared.model,
        prompt: request.prompt,
        custom_llm_provider: prepared.provider,
        litellm_params,
        optional_params: prepared.params,
        extra_headers: prepared.extra_headers,
        extra_body: prepared.extra_body,
        timeout: prepared.timeout,
        client: client.map(|transport| SuppliedClient {
            kind: if is_async {
                ClientKind::Asynchronous
            } else {
                ClientKind::Sync
            },
            transport,
        }),
    }
}

// The sync path polls once because a non-pending handler must answer immediately.
pub(super) fn poll_once<Running>(future: Running) -> Polled<Running::Output>
where
    Running: Future,
{
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Polled::Ready(outcome),
        Poll::Pending => Polled::StillWaiting,
    }
}
