use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};

use crate::websocket_oracle::{Action, Direction, Frame, MAX_FRAME_BYTES, WebSocketCase};

use super::{DriverError, HandshakeObservation, TranscriptObservation};

type GatewaySocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn exchange(
    base_url: &str,
    case: &WebSocketCase,
) -> Result<TranscriptObservation, DriverError> {
    let deadline = Duration::from_secs(if case.execution_seconds == 0 {
        120
    } else {
        case.execution_seconds
    });
    timeout(deadline, exchange_bounded(base_url, case))
        .await
        .map_err(|_| DriverError::Transport("WebSocket transcript timed out".to_owned()))?
}

async fn exchange_bounded(
    base_url: &str,
    case: &WebSocketCase,
) -> Result<TranscriptObservation, DriverError> {
    let endpoint = format!("{}{}", base_url.trim_end_matches('/'), case.path)
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let mut request = endpoint
        .into_client_request()
        .map_err(|error| DriverError::Transport(error.to_string()))?;
    for (name, value) in &case.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| DriverError::Transport(error.to_string()))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| DriverError::Transport(error.to_string()))?;
        request.headers_mut().insert(name, value);
    }
    if !case.requested_subprotocols.is_empty() {
        let protocols = case.requested_subprotocols.join(", ");
        let value = HeaderValue::from_str(&protocols)
            .map_err(|error| DriverError::Transport(error.to_string()))?;
        request
            .headers_mut()
            .insert("sec-websocket-protocol", value);
    }
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_FRAME_BYTES);
    config.max_frame_size = Some(MAX_FRAME_BYTES);
    config.max_write_buffer_size = MAX_FRAME_BYTES
        .checked_add(config.write_buffer_size)
        .ok_or_else(|| DriverError::Transport("write buffer limit overflow".to_owned()))?;
    let (mut socket, response) = match connect_async_with_config(request, Some(config), false).await
    {
        Ok(connection) => connection,
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            return Ok(TranscriptObservation {
                handshake: HandshakeObservation {
                    status: response.status().as_u16(),
                    negotiated_subprotocol: None,
                },
                frames: Vec::new(),
                close: None,
            });
        }
        Err(error) => return Err(DriverError::Transport(error.to_string())),
    };
    let handshake = HandshakeObservation {
        status: response.status().as_u16(),
        negotiated_subprotocol: response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    };
    let mut frames = Vec::new();
    let mut provider_count = 0usize;
    let mut provider_order_index = 0usize;
    let mut provider_observations = Vec::new();
    for frame in &case.frames {
        match (frame.direction, frame.action) {
            (Direction::ClientToProvider, Action::Send) => {
                if let Some(barrier) = &frame.barrier {
                    let provider_frames = case
                        .frames
                        .iter()
                        .filter(|provider_frame| {
                            provider_frame.direction == Direction::ProviderToClient
                        })
                        .collect::<Vec<_>>();
                    let target = provider_frames
                        .iter()
                        .position(|provider_frame| provider_frame.barrier.as_ref() == Some(barrier))
                        .ok_or_else(|| {
                            DriverError::Transport(format!("unknown barrier {barrier}"))
                        })?;
                    while provider_count <= target {
                        let expected = provider_frames.get(provider_count).ok_or_else(|| {
                            DriverError::Transport("provider frame sequence ended".to_owned())
                        })?;
                        let observed = receive_provider_frame(&mut socket, expected).await?;
                        if let Some(name) = &expected.barrier {
                            provider_observations.push(name.clone());
                        }
                        frames.push(observed);
                        provider_count = provider_count.checked_add(1).ok_or_else(|| {
                            DriverError::Transport("provider count overflow".to_owned())
                        })?;
                    }
                    if !provider_observations
                        .iter()
                        .any(|observed| observed == barrier)
                    {
                        return Err(DriverError::Transport(format!(
                            "barrier {barrier} was not observed"
                        )));
                    }
                }
                send_client_frame(&mut socket, frame).await?;
                frames.push(frame.clone());
            }
            (Direction::ProviderToClient, Action::Expect) => {
                if provider_order_index < provider_count {
                    provider_order_index =
                        provider_order_index.checked_add(1).ok_or_else(|| {
                            DriverError::Transport("provider index overflow".to_owned())
                        })?;
                    continue;
                }
                let provider_frames = case
                    .frames
                    .iter()
                    .filter(|provider_frame| {
                        provider_frame.direction == Direction::ProviderToClient
                    })
                    .collect::<Vec<_>>();
                if provider_count < provider_frames.len() {
                    let expected = provider_frames.get(provider_count).ok_or_else(|| {
                        DriverError::Transport("provider frame sequence ended".to_owned())
                    })?;
                    let observed = receive_provider_frame(&mut socket, expected).await?;
                    if let Some(name) = &expected.barrier {
                        provider_observations.push(name.clone());
                    }
                    frames.push(observed);
                    provider_count = provider_count.checked_add(1).ok_or_else(|| {
                        DriverError::Transport("provider count overflow".to_owned())
                    })?;
                    provider_order_index =
                        provider_order_index.checked_add(1).ok_or_else(|| {
                            DriverError::Transport("provider index overflow".to_owned())
                        })?;
                }
            }
            (Direction::ClientToProvider, Action::Expect)
            | (Direction::ProviderToClient, Action::Send) => {}
        }
    }
    let close = match &case.close {
        Some(expected) if expected.initiator == "client" => {
            let frame = CloseFrame {
                code: CloseCode::from(expected.code),
                reason: std::borrow::Cow::Owned(expected.reason.clone()),
            };
            socket
                .send(Message::Close(Some(frame)))
                .await
                .map_err(|error| DriverError::Transport(error.to_string()))?;
            Some(expected.clone())
        }
        Some(expected) if expected.initiator == "provider" => {
            Some(receive_close(&mut socket).await?)
        }
        Some(_) => return Err(DriverError::Transport("invalid close initiator".to_owned())),
        None => None,
    };
    Ok(TranscriptObservation {
        handshake,
        frames,
        close,
    })
}

async fn send_client_frame(socket: &mut GatewaySocket, frame: &Frame) -> Result<(), DriverError> {
    let message =
        match &frame.text {
            Some(text) => Message::Text(text.clone()),
            None => Message::Binary(frame.binary_data.clone().ok_or_else(|| {
                DriverError::Transport("binary frame fixture is missing".to_owned())
            })?),
        };
    socket
        .send(message)
        .await
        .map_err(|error| DriverError::Transport(error.to_string()))
}

async fn receive_provider_frame(
    socket: &mut GatewaySocket,
    expected: &Frame,
) -> Result<Frame, DriverError> {
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(|| DriverError::Transport("provider socket ended".to_owned()))?
            .map_err(|error| DriverError::Transport(error.to_string()))?;
        match message {
            Message::Text(text) => {
                return Ok(Frame {
                    direction: Direction::ProviderToClient,
                    action: Action::Expect,
                    text: Some(text),
                    binary: None,
                    binary_data: None,
                    barrier: expected.barrier.clone(),
                });
            }
            Message::Binary(bytes) => {
                return Ok(Frame {
                    direction: Direction::ProviderToClient,
                    action: Action::Expect,
                    text: None,
                    binary: expected.binary.clone(),
                    binary_data: Some(bytes),
                    barrier: expected.barrier.clone(),
                });
            }
            Message::Ping(payload) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| DriverError::Transport(error.to_string()))?,
            Message::Pong(_) | Message::Frame(_) => {}
            Message::Close(_) => {
                return Err(DriverError::Transport(
                    "provider closed before expected frame".to_owned(),
                ));
            }
        }
    }
}

async fn receive_close(
    socket: &mut GatewaySocket,
) -> Result<crate::websocket_oracle::Close, DriverError> {
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(|| DriverError::Transport("provider socket ended before close".to_owned()))?
            .map_err(|error| DriverError::Transport(error.to_string()))?;
        if let Message::Close(Some(frame)) = message {
            return Ok(crate::websocket_oracle::Close {
                initiator: "provider".to_owned(),
                code: u16::from(frame.code),
                reason: frame.reason.into_owned(),
            });
        }
    }
}
