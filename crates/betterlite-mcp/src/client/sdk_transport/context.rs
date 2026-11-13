use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

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

pub enum SdkTransport {
    Streamable(StreamableHttp),
    LegacySse(LegacySse),
    Stdio(StdioTransport),
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

impl TransportContext for SdkTransport {
    type Streams = SdkStreams;
    type Error = SdkError;

    fn enter(&mut self) -> impl Future<Output = Result<SdkStreams, SdkError>> + Send {
        async move {
            match self {
                Self::Streamable(transport) => {
                    let streams = transport.enter().await?;
                    Ok(SdkStreams::Streamable(streams))
                }
                Self::LegacySse(transport) => {
                    let streams = transport.enter().await?;
                    Ok(SdkStreams::LegacySse(streams))
                }
                Self::Stdio(transport) => {
                    let streams = transport
                        .enter()
                        .await
                        .map_err(|error| SdkError::Io(Arc::new(error)))?;
                    Ok(SdkStreams::Stdio(StdioRw::new_client(
                        streams.stdout,
                        streams.stdin,
                    )))
                }
            }
        }
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        async move {
            match self {
                Self::Streamable(transport) => transport.exit().await,
                Self::LegacySse(transport) => transport.exit().await,
                Self::Stdio(transport) => transport.exit().await.map_err(stdio_exit),
            }
        }
    }
}

impl Transport<RoleClient> for SdkStreams {
    type Error = SdkError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        let future: Pin<Box<dyn Future<Output = Result<(), SdkError>> + Send>> = match self {
            Self::Streamable(streams) => Box::pin(streams.send(item)),
            Self::LegacySse(streams) => Box::pin(streams.send(item)),
            Self::Stdio(streams) => Box::pin(streams.send(item)),
        };
        future
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        async move {
            match self {
                Self::Streamable(streams) => streams.receive().await,
                Self::LegacySse(streams) => streams.receive().await,
                Self::Stdio(streams) => streams.receive().await,
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> + Send {
        async move {
            match self {
                Self::Streamable(streams) => streams.close().await,
                Self::LegacySse(streams) => streams.close().await,
                Self::Stdio(streams) => streams.close().await,
            }
        }
    }
}
