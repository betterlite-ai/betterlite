use crate::error_body::request_timeout_error;
use crate::routes::router;
use crate::shutdown::build_shutdown_signal;
use axum::error_handling::HandleErrorLayer;
use axum::serve::Listener;
use betterlite_config::types::Config;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::task::{Context, Poll};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::sleep;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;

const MAX_CONNECTIONS: usize = 1024;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to register {signal} handler: {source}")]
    SignalRegistration {
        signal: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("failed to bind {address}: {source}")]
    Bind {
        address: String,
        #[source]
        source: io::Error,
    },
}

pub async fn run(config: Config, host: &str, port: u16) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
    let address = format!("{host}:{port}");
    let tcp_listener = TcpListener::bind((host, port))
        .await
        .map_err(|source| ServerError::Bind { address, source })?;
    tracing::info!(host, port, "betterlite server listening");
    let shutting_down = Arc::new(AtomicBool::new(false));
    let application = router(
        config.general_settings.master_key,
        Arc::clone(&shutting_down),
    )
    .layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
                request_timeout_error()
            }))
            .layer(TimeoutLayer::new(Duration::from_secs(30))),
    );
    let bounded_listener = BoundedListener {
        listener: tcp_listener,
        permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
    };
    axum::serve(bounded_listener, application)
        .with_graceful_shutdown(shutdown_signal.wait(shutting_down))
        .await
        .map_err(|source| ServerError::Bind {
            address: format!("{host}:{port}"),
            source,
        })
}

struct BoundedListener {
    listener: TcpListener,
    permits: Arc<Semaphore>,
}

impl Listener for BoundedListener {
    type Io = BoundedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok(permit) = Arc::clone(&self.permits).acquire_owned().await else {
                continue;
            };
            match self.listener.accept().await {
                Ok((stream, address)) => {
                    return (
                        BoundedStream {
                            stream,
                            _permit: permit,
                        },
                        address,
                    );
                }
                Err(error) => {
                    drop(permit);
                    tracing::error!(%error, "failed to accept connection");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

struct BoundedStream {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for BoundedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for BoundedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
