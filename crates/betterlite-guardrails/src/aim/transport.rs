use std::time::Duration;

use async_trait::async_trait;
use futures_util::SinkExt as _;
use futures_util::stream::{SplitSink, SplitStream, StreamExt as _};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};

use super::error::AimError;
use super::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, AimSocketTimeouts,
    MAX_STREAM_FRAME_BYTES,
};

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct AimHttpResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait AimPoster: Send + Sync {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError>;
}

pub struct HttpPoster {
    pub client: reqwest::Client,
}

#[async_trait]
impl AimPoster for HttpPoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut request = self
            .client
            .post(url)
            .headers(headers.clone())
            .body(body.to_vec());
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let mut response = request.send().await.map_err(request_error)?;
        let status = response.status();
        let mut collected = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if collected.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AimError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            collected.extend_from_slice(&chunk);
        }
        Ok(AimHttpResponse {
            status,
            body: collected,
        })
    }
}

fn request_error(source: reqwest::Error) -> AimError {
    if source.is_timeout() {
        AimError::Timeout { source }
    } else {
        AimError::Request { source }
    }
}

type SocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct AimWebSocketConnector {
    pub max_frame_bytes: usize,
}

impl Default for AimWebSocketConnector {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_STREAM_FRAME_BYTES,
        }
    }
}

#[async_trait]
impl AimSocketConnector for AimWebSocketConnector {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeouts: AimSocketTimeouts,
    ) -> Result<AimSocket, AimError> {
        let mut request = url
            .into_client_request()
            .map_err(|source| AimError::WebSocketRequest { source })?;
        for (name, value) in headers {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(self.max_frame_bytes))
            .max_frame_size(Some(self.max_frame_bytes));
        let opening = connect_async_with_config(request, Some(config), false);
        let (socket, _response) = tokio::time::timeout(timeouts.open, opening)
            .await
            .map_err(|_elapsed| AimError::ConnectTimeout)?
            .map_err(|source| AimError::Connect { source })?;
        let (writer, reader) = socket.split();
        Ok(AimSocket {
            reader: Box::new(TungsteniteReader {
                stream: reader,
                read_timeout: timeouts.read,
            }),
            writer: Box::new(TungsteniteWriter { sink: writer }),
        })
    }
}

struct TungsteniteReader {
    stream: SplitStream<SocketStream>,
    read_timeout: Duration,
}

#[async_trait]
impl AimFrameReader for TungsteniteReader {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        loop {
            let next = tokio::time::timeout(self.read_timeout, self.stream.next())
                .await
                .map_err(|_elapsed| AimError::StreamTimeout)?;
            match next.map_err(|source| AimError::WebSocket { source })? {
                Message::Text(text) => return Ok(Some(text.as_str().to_owned())),
                Message::Binary(bytes) => {
                    return String::from_utf8(bytes.to_vec())
                        .map(Some)
                        .map_err(|source| AimError::FrameUtf8 { source });
                }
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }
}

struct TungsteniteWriter {
    sink: SplitSink<SocketStream, Message>,
}

#[async_trait]
impl AimFrameWriter for TungsteniteWriter {
    async fn send_text(&mut self, text: String) -> Result<(), AimError> {
        self.sink
            .send(Message::Text(Utf8Bytes::from(text)))
            .await
            .map_err(|source| AimError::WebSocket { source })
    }
}
