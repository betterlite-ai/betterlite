use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    Error,
    client::IntoClientRequest as _,
    handshake::client::{Request, Response},
    http::{HeaderName, HeaderValue},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};

use super::{
    Clock, DriverError, HandshakeObservation, TranscriptObservation,
    socket::{Peer, config},
};
use crate::websocket_oracle::{
    Direction, OracleLimits, RealtimeSettings, WebSocketBounds, WebSocketCase,
};

pub trait Transport: Send + Sync {
    fn connect(
        &self,
        request: Request,
        limits: &OracleLimits,
        settings: &RealtimeSettings,
    ) -> impl Future<Output = Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>> + Send;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokioTransport;

impl Transport for TokioTransport {
    async fn connect(
        &self,
        request: Request,
        limits: &OracleLimits,
        settings: &RealtimeSettings,
    ) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
        connect_async_with_config(request, Some(config(limits, settings)), false).await
    }
}

pub async fn exchange<C, T>(
    base_url: &str,
    case: &WebSocketCase,
    bounds: &WebSocketBounds,
    clock: &C,
    transport: &T,
) -> Result<TranscriptObservation, DriverError>
where
    C: Clock,
    T: Transport,
{
    clock
        .timeout(
            Duration::from_secs(case.execution_seconds),
            exchange_bounded(base_url, case, &bounds.limits, &bounds.settings, transport),
        )
        .await?
}

fn request(base_url: &str, case: &WebSocketCase) -> Result<Request, DriverError> {
    let endpoint = format!("{}{}", base_url.trim_end_matches('/'), case.path)
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let mut request = endpoint.into_client_request()?;
    let mut seen = HashSet::new();
    for (name, value) in &case.headers {
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(DriverError::Transport(format!(
                "duplicate header name: {name}"
            )));
        }
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    if !case.requested_subprotocols.is_empty() {
        if seen.contains("sec-websocket-protocol") {
            return Err(DriverError::Transport(
                "sec-websocket-protocol set both explicitly and via requested_subprotocols"
                    .to_owned(),
            ));
        }
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_str(&case.requested_subprotocols.join(", "))?,
        );
    }
    Ok(request)
}

async fn exchange_bounded<T>(
    base_url: &str,
    case: &WebSocketCase,
    limits: &OracleLimits,
    settings: &RealtimeSettings,
    transport: &T,
) -> Result<TranscriptObservation, DriverError>
where
    T: Transport,
{
    let (stream, response) = match transport
        .connect(request(base_url, case)?, limits, settings)
        .await
    {
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
            .transpose()?,
    };
    let mut socket = Peer {
        stream,
        bytes: 0,
        count: 0,
        limits: *limits,
    };
    let (frames, close) = socket.play(case, Direction::ClientToProvider).await?;
    Ok(TranscriptObservation {
        handshake,
        frames,
        close,
    })
}
