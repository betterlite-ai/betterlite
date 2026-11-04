use std::net::SocketAddr;

use futures_util::SinkExt;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::websocket_oracle::{Action, Close, Direction, Frame, MAX_FRAME_BYTES, WebSocketCase};

use super::DriverError;
use super::socket::Peer;

pub(crate) struct ProviderHandle {
    pub(crate) address: SocketAddr,
    tasks: JoinSet<Result<ProviderObservation, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderObservation {
    pub(crate) connection_count: usize,
    pub(crate) request_uri: String,
    pub(crate) frames: Vec<Frame>,
    pub(crate) close: Option<Close>,
}

pub(crate) async fn start(case: WebSocketCase) -> Result<ProviderHandle, DriverError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| DriverError::Transport(error.to_string()))?;
    let address = listener
        .local_addr()
        .map_err(|error| DriverError::Transport(error.to_string()))?;
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { serve(listener, case).await });
    Ok(ProviderHandle { address, tasks })
}

impl ProviderHandle {
    pub(crate) async fn finish(
        mut self,
        deadline: std::time::Duration,
    ) -> Result<ProviderObservation, DriverError> {
        match timeout(deadline, self.tasks.join_next()).await {
            Ok(Some(Ok(Ok(observation)))) => Ok(observation),
            Ok(Some(Ok(Err(error)))) => Err(DriverError::Transport(error)),
            Ok(Some(Err(error))) => Err(DriverError::Transport(error.to_string())),
            Ok(None) => Err(DriverError::Transport(
                "provider task is missing".to_owned(),
            )),
            Err(_) => {
                self.tasks.shutdown().await;
                Err(DriverError::Transport(
                    "provider transcript timed out".to_owned(),
                ))
            }
        }
    }

    pub(crate) async fn cancel(mut self) -> Result<(), DriverError> {
        self.tasks.abort_all();
        while let Some(result) = self.tasks.join_next().await {
            match result {
                Ok(outcome) => {
                    outcome.map_err(DriverError::Transport)?;
                }
                Err(error) if error.is_cancelled() => {}
                Err(error) => return Err(DriverError::Transport(error.to_string())),
            }
        }
        Ok(())
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
    let stream = match accept_hdr_async_with_config(stream, callback, Some(config)).await {
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
    let mut socket = Peer { stream };
    let mut observed_frames = Vec::new();
    for frame in &case.frames {
        match (frame.direction, frame.action) {
            (Direction::ClientToProvider, Action::Send) => {
                let observed = Frame::try_from((
                    socket.receive().await.map_err(|error| error.to_string())?,
                    frame,
                ))
                .map_err(|error| error.to_string())?;
                observed_frames.push(observed);
            }
            (Direction::ProviderToClient, Action::Expect) => {
                socket
                    .stream
                    .send(Message::try_from(frame).map_err(|error| error.to_string())?)
                    .await
                    .map_err(|error| error.to_string())?;
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
                .stream
                .send(Message::Close(Some(close)))
                .await
                .map_err(|error| error.to_string())?;
            Some(expected.clone())
        }
        Some(expected) if expected.initiator == "client" => Some(
            Close::try_from((
                socket.receive().await.map_err(|error| error.to_string())?,
                "client",
            ))
            .map_err(|error| error.to_string())?,
        ),
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
