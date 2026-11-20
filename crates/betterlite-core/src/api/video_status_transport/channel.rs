use std::future::{Future, ready};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;

use super::clients::{AsyncClient, ClientFactories, SyncClient, provider_id, ssl_verify};
use super::config::StatusConfig;
use super::error::{ProviderFailure, VideoStatusError};
use super::records::{Params, StatusCall, StatusRequest, SuppliedClient, VideoReply};
use crate::api::video_records::VideoObject;

// What differs between the blocking and the async call, so one body serves both
pub(crate) trait Channel<Factories: ClientFactories> {
    type Client: Send + Sync;

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Self::Client>;

    fn build(
        factories: &Factories,
        provider: &str,
        litellm_params: &Params,
    ) -> Result<Self::Client, VideoStatusError>;

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

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Factories::Synchronous> {
        match call.client.as_ref() {
            Some(SuppliedClient::Sync(client)) => Some(*client),
            Some(SuppliedClient::Async(_)) | None => None,
        }
    }

    fn build(
        factories: &Factories,
        _provider: &str,
        litellm_params: &Params,
    ) -> Result<Self::Client, VideoStatusError> {
        let verify = ssl_verify(litellm_params)?;
        factories
            .sync_client(verify)
            .map_err(VideoStatusError::from)
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

    fn supplied<'call>(
        call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Option<&'call Factories::Asynchronous> {
        match call.client.as_ref() {
            Some(SuppliedClient::Async(client)) => Some(*client),
            Some(SuppliedClient::Sync(_)) | None => None,
        }
    }

    fn build(
        factories: &Factories,
        provider: &str,
        litellm_params: &Params,
    ) -> Result<Self::Client, VideoStatusError> {
        let verified = provider_id(provider)?;
        let verify = ssl_verify(litellm_params)?;
        factories
            .async_client(verified, verify)
            .map_err(VideoStatusError::from)
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
