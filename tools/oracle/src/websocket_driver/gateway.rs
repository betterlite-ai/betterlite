use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::{
    Error,
    client::IntoClientRequest as _,
    handshake::client::Request,
    http::{HeaderName, HeaderValue},
};

use super::{
    DriverError, HandshakeObservation, TranscriptObservation,
    socket::{Peer, config},
};
use crate::websocket_oracle::{Direction, WebSocketCase};

pub async fn exchange(
    base_url: &str,
    case: &WebSocketCase,
) -> Result<TranscriptObservation, DriverError> {
    timeout(
        Duration::from_secs(case.execution_seconds),
        exchange_bounded(base_url, case),
    )
    .await?
}

fn request(base_url: &str, case: &WebSocketCase) -> Result<Request, DriverError> {
    let endpoint = format!("{}{}", base_url.trim_end_matches('/'), case.path)
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let mut request = endpoint.into_client_request()?;
    for (name, value) in &case.headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    if !case.requested_subprotocols.is_empty() {
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_str(&case.requested_subprotocols.join(", "))?,
        );
    }
    Ok(request)
}

async fn exchange_bounded(
    base_url: &str,
    case: &WebSocketCase,
) -> Result<TranscriptObservation, DriverError> {
    let (stream, response) =
        match connect_async_with_config(request(base_url, case)?, Some(config()), false).await {
            Ok(connection) => connection,
            Err(Error::Http(response)) => {
                return Ok(TranscriptObservation {
                    handshake: HandshakeObservation {
                        status: response.status().as_u16(),
                        negotiated_subprotocol: None,
                    },
                    frames: Vec::new(),
                    close: None,
                });
            }
            Err(source) => return Err(DriverError::from(source)),
        };
    let handshake = HandshakeObservation {
        status: response.status().as_u16(),
        negotiated_subprotocol: response
            .headers()
            .get("sec-websocket-protocol")
            .map(|value| value.to_str().map(str::to_owned))
            .transpose()
            .map_err(|source| {
                DriverError::Transport(format!("invalid negotiated protocol: {source}"))
            })?,
    };
    let mut socket = Peer {
        stream,
        bytes: 0,
        count: 0,
    };
    let (frames, close) = socket.play(case, Direction::ClientToProvider).await?;
    Ok(TranscriptObservation {
        handshake,
        frames,
        close,
    })
}
