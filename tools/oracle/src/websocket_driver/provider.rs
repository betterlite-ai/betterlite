use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{WebSocketStream, accept_hdr_async_with_config};

use crate::websocket_oracle::{Action, Close, Direction, Frame, MAX_FRAME_BYTES, WebSocketCase};

use super::DriverError;

pub struct ProviderHandle {
    pub address: SocketAddr,
    task: JoinHandle<Result<ProviderObservation, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderObservation {
    pub connection_count: usize,
    pub request_uri: String,
    pub frames: Vec<Frame>,
    pub close: Option<Close>,
}

pub async fn start(case: WebSocketCase) -> Result<ProviderHandle, DriverError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| DriverError::Transport(error.to_string()))?;
    let address = listener
        .local_addr()
        .map_err(|error| DriverError::Transport(error.to_string()))?;
    let task = tokio::spawn(async move { serve(listener, case).await });
    Ok(ProviderHandle { address, task })
}

impl ProviderHandle {
    pub async fn finish(
        mut self,
        deadline: std::time::Duration,
    ) -> Result<ProviderObservation, DriverError> {
        match timeout(deadline, &mut self.task).await {
            Ok(Ok(Ok(observation))) => Ok(observation),
            Ok(Ok(Err(error))) => Err(DriverError::Transport(error)),
            Ok(Err(error)) => Err(DriverError::Transport(error.to_string())),
            Err(_) => {
                self.task.abort();
                let _result = self.task.await;
                Err(DriverError::Transport(
                    "provider transcript timed out".to_owned(),
                ))
            }
        }
    }

    pub async fn cancel(self) -> Result<(), DriverError> {
        self.task.abort();
        match self.task.await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(DriverError::Transport(error)),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(DriverError::Transport(error.to_string())),
        }
    }
}

async fn serve(listener: TcpListener, case: WebSocketCase) -> Result<ProviderObservation, String> {
    let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
    let request_uri = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_uri = std::sync::Arc::clone(&request_uri);
    let upstream_status = case.upstream_status;
    let callback = move |request: &Request, response: Response| {
        if let Ok(mut uri) = captured_uri.lock() {
            *uri = request.uri().to_string();
        }
        if let Some(status) = upstream_status {
            if status != 101 {
                let mut refusal = ErrorResponse::new(Some("upstream refused".to_owned()));
                let status = StatusCode::from_u16(status).map_err(|_| refusal.clone())?;
                *refusal.status_mut() = status;
                return Err(refusal);
            }
        }
        Ok(response)
    };
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_FRAME_BYTES);
    config.max_frame_size = Some(MAX_FRAME_BYTES);
    let mut socket = match accept_hdr_async_with_config(stream, callback, Some(config)).await {
        Ok(socket) => socket,
        Err(tokio_tungstenite::tungstenite::Error::Http(_)) => {
            return Ok(ProviderObservation {
                connection_count: 1,
                request_uri: request_uri
                    .lock()
                    .map_err(|_| "provider request lock poisoned".to_owned())?
                    .clone(),
                frames: Vec::new(),
                close: None,
            });
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut observed_frames = Vec::new();
    for frame in &case.frames {
        match (frame.direction, frame.action) {
            (Direction::ClientToProvider, Action::Send) => {
                let observed = receive_client_frame(&mut socket, frame).await?;
                observed_frames.push(observed);
            }
            (Direction::ProviderToClient, Action::Expect) => {
                send_provider_frame(&mut socket, frame).await?;
                observed_frames.push(frame.clone());
            }
            (Direction::ClientToProvider, Action::Expect)
            | (Direction::ProviderToClient, Action::Send) => {}
        }
    }
    let close = match &case.close {
        Some(expected) if expected.initiator == "provider" => {
            let close = CloseFrame {
                code: CloseCode::from(expected.code),
                reason: std::borrow::Cow::Owned(expected.reason.clone()),
            };
            socket
                .send(Message::Close(Some(close)))
                .await
                .map_err(|error| error.to_string())?;
            Some(expected.clone())
        }
        Some(expected) if expected.initiator == "client" => Some(receive_close(&mut socket).await?),
        Some(_) => return Err("invalid close initiator".to_owned()),
        None => None,
    };
    let uri = request_uri
        .lock()
        .map_err(|_| "provider request lock poisoned".to_owned())?
        .clone();
    Ok(ProviderObservation {
        connection_count: 1,
        request_uri: uri,
        frames: observed_frames,
        close,
    })
}

async fn receive_client_frame<S>(
    socket: &mut WebSocketStream<S>,
    expected: &Frame,
) -> Result<Frame, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(|| "client socket ended before expected frame".to_owned())?
            .map_err(|error| error.to_string())?;
        match message {
            Message::Text(text) => {
                return Ok(Frame {
                    direction: Direction::ClientToProvider,
                    action: Action::Send,
                    text: Some(text),
                    binary: None,
                    binary_data: None,
                    barrier: expected.barrier.clone(),
                });
            }
            Message::Binary(bytes) => {
                return Ok(Frame {
                    direction: Direction::ClientToProvider,
                    action: Action::Send,
                    text: None,
                    binary: expected.binary.clone(),
                    binary_data: Some(bytes),
                    barrier: expected.barrier.clone(),
                });
            }
            Message::Ping(payload) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| error.to_string())?,
            Message::Pong(_) | Message::Frame(_) => {}
            Message::Close(_) => return Err("client closed before expected frame".to_owned()),
        }
    }
}

async fn send_provider_frame<S>(
    socket: &mut WebSocketStream<S>,
    frame: &Frame,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let message = match &frame.text {
        Some(text) => Message::Text(text.clone()),
        None => Message::Binary(
            frame
                .binary_data
                .clone()
                .ok_or_else(|| "provider binary fixture is missing".to_owned())?,
        ),
    };
    socket
        .send(message)
        .await
        .map_err(|error| error.to_string())
}

async fn receive_close<S>(socket: &mut WebSocketStream<S>) -> Result<Close, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(|| "client socket ended before close".to_owned())?
            .map_err(|error| error.to_string())?;
        if let Message::Close(Some(frame)) = message {
            return Ok(Close {
                initiator: "client".to_owned(),
                code: u16::from(frame.code),
                reason: frame.reason.into_owned(),
            });
        }
    }
}
