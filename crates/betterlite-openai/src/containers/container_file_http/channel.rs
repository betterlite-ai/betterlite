use std::future::{Future, ready};

use betterlite_http::proxy::pool::PoolError;

use super::ContainerFileHttpCall;
use super::request::SslVerify;
use super::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};

// What differs between the blocking and the async path, so one body serves both
pub trait Channel<Source: ClientSource> {
    type Client: Send + Sync;

    fn supplied<'client>(
        call: &ContainerFileHttpCall<'client, Source>,
    ) -> Option<&'client Self::Client>;

    fn build(source: &Source, ssl_verify: Option<&SslVerify>) -> Result<Self::Client, PoolError>;

    fn send(
        client: &Self::Client,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send;
}

pub struct BlockingKind;

pub struct AsyncKind;

impl<Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>> Channel<Source>
    for BlockingKind
{
    type Client = Source::Blocking;

    fn supplied<'client>(
        call: &ContainerFileHttpCall<'client, Source>,
    ) -> Option<&'client Source::Blocking> {
        call.blocking_client
    }

    fn build(
        source: &Source,
        ssl_verify: Option<&SslVerify>,
    ) -> Result<Source::Blocking, PoolError> {
        source.blocking_client(ssl_verify)
    }

    // the blocking client answers before this future is first polled, so it never suspends
    fn send(
        client: &Source::Blocking,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        ready(BlockingClient::send(client, request))
    }
}

impl<Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>> Channel<Source>
    for AsyncKind
{
    type Client = Source::Asynchronous;

    fn supplied<'client>(
        call: &ContainerFileHttpCall<'client, Source>,
    ) -> Option<&'client Source::Asynchronous> {
        call.async_client
    }

    fn build(
        source: &Source,
        ssl_verify: Option<&SslVerify>,
    ) -> Result<Source::Asynchronous, PoolError> {
        source.async_client(ssl_verify)
    }

    fn send(
        client: &Source::Asynchronous,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        AsyncClient::send(client, request)
    }
}
