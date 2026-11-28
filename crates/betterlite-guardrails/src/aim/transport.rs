use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::websocket_client::connect;
use betterlite_http::websocket_client::connection::{
    WebSocketConnection, WebSocketReceive, WebSocketReceiver, WebSocketSender,
};
use betterlite_http::websocket_client::error::WebSocketClientError;
use betterlite_http::websocket_client::request::WebSocketRequest;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use tokio::time;
use url::Url;

use super::error::AimError;
use super::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, MAX_STREAM_FRAME_BYTES,
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
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_vec());
        if let Some(limit) = timeout {
            request = request.timeout(limit);
        }
        let mut response = request.send().await.map_err(request_error)?;
        let status = response.status();
        let mut collected = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if collected.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AimError::ResponseTooLarge(MAX_RESPONSE_BYTES));
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
        AimError::Timeout(source)
    } else {
        AimError::Request(source)
    }
}

pub struct AimWebSocketConnector;

#[async_trait]
impl AimSocketConnector for AimWebSocketConnector {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        open: Duration,
        read: Duration,
    ) -> Result<AimSocket, AimError> {
        let parsed =
            Url::parse(url).map_err(|_error| AimError::WebSocket(WebSocketClientError::Url))?;
        let mut pairs = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            pairs.push((name.clone(), value.clone()));
        }
        let connection = connect(WebSocketRequest {
            url: parsed,
            headers: &pairs,
            tls: None,
            max_message_size: MAX_STREAM_FRAME_BYTES,
            open_timeout: open,
            keepalive: None,
            proxy: None,
        })
        .await
        .map_err(AimError::from)?;
        let WebSocketConnection { sender, receiver } = connection;
        Ok(AimSocket {
            reader: Box::new(ClientFrameReader {
                receiver,
                read_timeout: read,
            }),
            writer: Box::new(ClientFrameWriter { sender }),
        })
    }
}

struct ClientFrameReader {
    receiver: WebSocketReceiver,
    read_timeout: Duration,
}

#[async_trait]
impl AimFrameReader for ClientFrameReader {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        let next = time::timeout(self.read_timeout, self.receiver.recv())
            .await
            .map_err(|_elapsed| AimError::StreamTimeout)?;
        match next.map_err(AimError::WebSocket)? {
            WebSocketReceive::Text(text) => Ok(Some(text)),
            WebSocketReceive::Binary(bytes) => String::from_utf8(bytes.to_vec())
                .map(Some)
                .map_err(AimError::FrameUtf8),
            WebSocketReceive::Close(_) | WebSocketReceive::End => Ok(None),
        }
    }
}

struct ClientFrameWriter {
    sender: WebSocketSender,
}

#[async_trait]
impl AimFrameWriter for ClientFrameWriter {
    async fn send_text(&mut self, text: String) -> Result<(), AimError> {
        self.sender
            .send_text(&text)
            .await
            .map_err(AimError::WebSocket)
    }
}
