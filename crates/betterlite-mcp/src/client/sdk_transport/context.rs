use std::future::Future;
use std::io;
use std::sync::Arc;

use futures_util::FutureExt as _;
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
pub enum SdkStreams {
    Streamable(StreamableStreams),
    LegacySse(LegacySseStreams),
    Stdio(StdioRw),
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
    async fn enter(&mut self) -> Result<StdioRw, SdkError> {
        let streams = self
            .0
            .enter()
            .await
            .map_err(|error| SdkError::Io(Arc::new(error)))?;
        Ok(StdioRw::new_client(streams.stdout, streams.stdin))
    }
    async fn exit(self) -> Result<(), ExitFailure<SdkError>> {
        self.0.exit().await.map_err(stdio_exit)
    }
}
impl TransportContext for SdkTransport {
    type Streams = SdkStreams;
    type Error = SdkError;
    async fn enter(&mut self) -> Result<SdkStreams, SdkError> {
        match self {
            Self::Streamable(transport) => Ok(SdkStreams::Streamable(transport.enter().await?)),
            Self::LegacySse(transport) => Ok(SdkStreams::LegacySse(transport.enter().await?)),
            Self::Stdio(transport) => Ok(SdkStreams::Stdio(transport.enter().await?)),
        }
    }
    async fn exit(self) -> Result<(), ExitFailure<SdkError>> {
        match self {
            Self::Streamable(transport) => transport.exit().await,
            Self::LegacySse(transport) => transport.exit().await,
            Self::Stdio(transport) => transport.exit().await,
        }
    }
}
impl Transport<RoleClient> for SdkStreams {
    type Error = SdkError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        match self {
            Self::Streamable(streams) => streams.send(item).boxed(),
            Self::LegacySse(streams) => streams.send(item).boxed(),
            Self::Stdio(streams) => streams
                .send(item)
                .map(|sent| sent.map_err(|error| SdkError::Io(Arc::new(error))))
                .boxed(),
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        match self {
            Self::Streamable(streams) => streams.receive().await,
            Self::LegacySse(streams) => streams.receive().await,
            Self::Stdio(streams) => streams.receive().await,
        }
    }
    async fn close(&mut self) -> Result<(), SdkError> {
        match self {
            Self::Streamable(streams) => streams.close().await,
            Self::LegacySse(streams) => streams.close().await,
            Self::Stdio(streams) => streams
                .close()
                .await
                .map_err(|error| SdkError::Io(Arc::new(error))),
        }
    }
}
