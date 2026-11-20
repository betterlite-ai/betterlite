use std::future::{Future, ready};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::identity::provider_id::ProviderId;

use super::clients::{AsyncClient, ClientFactories, SyncClient, provider_id, ssl_verify};
use super::config::StatusConfig;
use super::error::{ProviderFailure, VideoStatusError};
use super::records::{Params, StatusCall, StatusRequest, VideoReply};
use crate::api::video_records::VideoObject;

// What differs between the blocking and the async call, so one body serves both
pub(crate) trait Channel<Factories: ClientFactories> {
    type Client: Send + Sync;
    // what this mode resolves the provider name into before it asks the factories
    type Provider;

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Self::Client>;

    fn resolve(provider: &str) -> Result<Self::Provider, VideoStatusError>;

    fn client(
        factories: &Factories,
        provider: Self::Provider,
        verify: Option<bool>,
    ) -> Result<Self::Client, PoolError>;

    // the provider is resolved first, as LiteLLM builds its enum before the client reads ssl_verify
    fn build(
        factories: &Factories,
        provider: &str,
        litellm_params: &Params,
    ) -> Result<Self::Client, VideoStatusError> {
        let resolved = Self::resolve(provider)?;
        let verify = ssl_verify(litellm_params)?;
        Self::client(factories, resolved, verify).map_err(VideoStatusError::from)
    }

    // one lifetime ties the client, the request and the returned future together
    fn send<'call>(
        client: &'call Self::Client,
        request: &'call StatusRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send + 'call;

    fn respond<'call>(
        config: &'call StatusConfig<'call, Factories>,
        reply: VideoReply<'call>,
        client: &'call Self::Client,
    ) -> impl Future<Output = Result<VideoObject, ProviderFailure>> + Send + 'call;
}

pub(crate) struct SynchronousKind;

pub(crate) struct AsynchronousKind;

impl<Factories: ClientFactories> Channel<Factories> for SynchronousKind {
    type Client = Factories::Synchronous;
    // the blocking factory is asked for no provider, so the name resolves to nothing
    type Provider = ();

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Factories::Synchronous> {
        call.client.as_ref()?.synchronous()
    }

    fn resolve(_provider: &str) -> Result<(), VideoStatusError> {
        Ok(())
    }

    fn client(
        factories: &Factories,
        _provider: (),
        verify: Option<bool>,
    ) -> Result<Self::Client, PoolError> {
        factories.sync_client(verify)
    }

    // the blocking client answers before this future is first polled, so it never suspends
    fn send<'call>(
        client: &'call Self::Client,
        request: &'call StatusRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send + 'call {
        ready(SyncClient::send(client, request))
    }

    fn respond<'call>(
        config: &'call StatusConfig<'call, Factories>,
        reply: VideoReply<'call>,
        client: &'call Self::Client,
    ) -> impl Future<Output = Result<VideoObject, ProviderFailure>> + Send + 'call {
        ready(config.transform_video_status_retrieve_response(reply, Some(client)))
    }
}

impl<Factories: ClientFactories> Channel<Factories> for AsynchronousKind {
    type Client = Factories::Asynchronous;
    // get_async_httpx_client takes the provider as a litellm.LlmProviders member
    type Provider = ProviderId;

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Factories::Asynchronous> {
        call.client.as_ref()?.asynchronous()
    }

    fn resolve(provider: &str) -> Result<ProviderId, VideoStatusError> {
        provider_id(provider)
    }

    fn client(
        factories: &Factories,
        provider: ProviderId,
        verify: Option<bool>,
    ) -> Result<Self::Client, PoolError> {
        factories.async_client(provider, verify)
    }

    fn send<'call>(
        client: &'call Self::Client,
        request: &'call StatusRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send + 'call {
        AsyncClient::send(client, request)
    }

    fn respond<'call>(
        config: &'call StatusConfig<'call, Factories>,
        reply: VideoReply<'call>,
        client: &'call Self::Client,
    ) -> impl Future<Output = Result<VideoObject, ProviderFailure>> + Send + 'call {
        config.async_transform_video_status_retrieve_response(reply, Some(client))
    }
}
