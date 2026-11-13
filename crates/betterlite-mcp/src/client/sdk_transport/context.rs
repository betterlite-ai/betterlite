use std::future::Future;
use std::io;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;

use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::stdio::StdioTransport;
use crate::client::sessions::transport::TransportContext;

use super::error::SdkError;
use super::legacy_sse::{LegacySse, LegacySseStreams};
use super::stdio::StdioRw;
use super::streamable::{StreamableHttp, StreamableStreams};

pub struct StdioContext(pub StdioTransport);
pub enum SdkTransport {
    Streamable(StreamableHttp),
    LegacySse(LegacySse),
    Stdio(StdioContext),
}
pub struct SdkStreams(Box<dyn BoxedStreams>);
trait BoxedStreams: Send {
    fn send_boxed(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> BoxFuture<'static, Result<(), SdkError>>;
    fn receive_boxed(&mut self) -> BoxFuture<'_, Option<RxJsonRpcMessage<RoleClient>>>;
    fn close_boxed(&mut self) -> BoxFuture<'_, Result<(), SdkError>>;
}

impl<Streams> BoxedStreams for Streams
where
    Streams: Transport<RoleClient, Error = SdkError> + Send,
{
    fn send_boxed(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> BoxFuture<'static, Result<(), SdkError>> {
        Box::pin(Transport::<RoleClient>::send(self, item))
    }
    fn receive_boxed(&mut self) -> BoxFuture<'_, Option<RxJsonRpcMessage<RoleClient>>> {
        Box::pin(Transport::<RoleClient>::receive(self))
    }
    fn close_boxed(&mut self) -> BoxFuture<'_, Result<(), SdkError>> {
        Box::pin(Transport::<RoleClient>::close(self))
    }
}
impl From<StreamableStreams> for SdkStreams {
    fn from(streams: StreamableStreams) -> Self {
        Self(Box::new(streams))
    }
}
impl From<LegacySseStreams> for SdkStreams {
    fn from(streams: LegacySseStreams) -> Self {
        Self(Box::new(streams))
    }
}
impl From<StdioRw> for SdkStreams {
    fn from(streams: StdioRw) -> Self {
        Self(Box::new(streams))
    }
}
fn stdio_exit(failure: ExitFailure<io::Error>) -> ExitFailure<SdkError> {
    match failure {
        ExitFailure::Cancelled => ExitFailure::Cancelled,
        ExitFailure::Failure(source) => ExitFailure::Failure(Arc::new(SdkError::Io(source))),
        ExitFailure::Group(group) => {
            ExitFailure::Group(group.into_iter().map(stdio_exit).collect())
        }
    }
}
impl TransportContext for StdioContext {
    type Streams = StdioRw;
    type Error = SdkError;
    fn enter(&mut self) -> impl Future<Output = Result<StdioRw, SdkError>> + Send {
        async move {
            let streams = self
                .0
                .enter()
                .await
                .map_err(|error| SdkError::Io(Arc::new(error)))?;
            Ok(StdioRw::new_client(streams.stdout, streams.stdin))
        }
    }
    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        async move { self.0.exit().await.map_err(stdio_exit) }
    }
}

macro_rules! open_transport {
    ($transport:expr) => {
        match $transport {
            SdkTransport::Streamable(inner) => SdkStreams::from(inner.enter().await?),
            SdkTransport::LegacySse(inner) => SdkStreams::from(inner.enter().await?),
            SdkTransport::Stdio(inner) => SdkStreams::from(inner.enter().await?),
        }
    };
}

macro_rules! close_transport {
    ($transport:expr) => {
        match $transport {
            SdkTransport::Streamable(inner) => inner.exit().await,
            SdkTransport::LegacySse(inner) => inner.exit().await,
            SdkTransport::Stdio(inner) => inner.exit().await,
        }
    };
}
impl TransportContext for SdkTransport {
    type Streams = SdkStreams;
    type Error = SdkError;
    fn enter(&mut self) -> impl Future<Output = Result<SdkStreams, SdkError>> + Send {
        async move { open_transport!(self) }
    }
    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        async move { close_transport!(self) }
    }
}
impl Transport<RoleClient> for SdkStreams {
    type Error = SdkError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        self.0.send_boxed(item)
    }
    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.0.receive_boxed()
    }
    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> + Send {
        self.0.close_boxed()
    }
}
