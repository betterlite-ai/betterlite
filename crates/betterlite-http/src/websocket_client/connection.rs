use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::Mutex;
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::tungstenite::error::{Error as TungsteniteError, ProtocolError};
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::error::WebSocketClientError;
use super::request::Keepalive;
use super::stream::AsyncStream;

pub const KEEPALIVE_CLOSE_CODE: u16 = 1011;
pub const KEEPALIVE_TIMEOUT_REASON: &str = "keepalive ping timeout";

// websockets' Close normalizes a close frame without a code to 1005 and an empty reason
pub const EMPTY_CLOSE_CODE: u16 = 1005;

type Socket = WebSocketStream<MaybeTlsStream<Box<dyn AsyncStream>>>;
type SocketSink = SplitSink<Socket, Message>;
type SocketStream = SplitStream<Socket>;

// a peer's close, reported with the code and reason websockets' recv raises
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerClose {
    pub code: u16,
    pub reason: String,
}

// what the receiving half answers: a data message, the peer's close, or the end of the stream without one
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketReceive {
    Text(String),
    Binary(Bytes),
    Close(PeerClose),
    End,
}

// the two halves over one split stream, sharing the sink behind one tokio Mutex so either can write
pub struct WebSocketConnection {
    pub sender: WebSocketSender,
    pub receiver: WebSocketReceiver,
}

impl WebSocketConnection {
    pub(crate) fn new(socket: Socket, keepalive: Option<Keepalive>) -> Self {
        let (sink, stream) = socket.split();
        let sink = Arc::new(Mutex::new(sink));
        let sender = WebSocketSender {
            sink: Arc::clone(&sink),
        };
        let receiver = WebSocketReceiver {
            stream,
            sender: sender.clone(),
            keepalive,
            next_ping: keepalive.map(|options| Instant::now() + options.interval),
            pong_deadline: None,
        };
        Self { sender, receiver }
    }
}

#[derive(Clone)]
pub struct WebSocketSender {
    sink: Arc<Mutex<SocketSink>>,
}

impl WebSocketSender {
    pub async fn send_text(&self, text: &str) -> Result<(), WebSocketClientError> {
        self.sink
            .lock()
            .await
            .send(Message::Text(Utf8Bytes::from(text)))
            .await
            .map_err(WebSocketClientError::from_tungstenite)
    }

    pub async fn send_binary(&self, bytes: &[u8]) -> Result<(), WebSocketClientError> {
        self.sink
            .lock()
            .await
            .send(Message::Binary(Bytes::copy_from_slice(bytes)))
            .await
            .map_err(WebSocketClientError::from_tungstenite)
    }

    pub async fn close(&self, code: u16, reason: &str) -> Result<(), WebSocketClientError> {
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: Utf8Bytes::from(reason),
        };
        self.sink
            .lock()
            .await
            .send(Message::Close(Some(frame)))
            .await
            .map_err(WebSocketClientError::from_tungstenite)
    }
}

// websockets' keepalive task runs beside recv: a ping every interval, and a socket that misses a pong fails with 1011
pub struct WebSocketReceiver {
    stream: SocketStream,
    sender: WebSocketSender,
    keepalive: Option<Keepalive>,
    next_ping: Option<Instant>,
    pong_deadline: Option<Instant>,
}

impl WebSocketReceiver {
    pub async fn recv(&mut self) -> Result<WebSocketReceive, WebSocketClientError> {
        loop {
            if let Some(options) = self.keepalive {
                let now = Instant::now();
                if self.pong_deadline.is_some_and(|deadline| now >= deadline) {
                    drop(
                        self.sender
                            .close(KEEPALIVE_CLOSE_CODE, KEEPALIVE_TIMEOUT_REASON)
                            .await,
                    );
                    return Err(WebSocketClientError::KeepaliveTimeout);
                }
                if self.pong_deadline.is_none() && self.next_ping.is_some_and(|next| now >= next) {
                    self.sender
                        .sink
                        .lock()
                        .await
                        .send(Message::Ping(Bytes::new()))
                        .await
                        .map_err(WebSocketClientError::from_tungstenite)?;
                    self.next_ping = Some(now + options.interval);
                    self.pong_deadline = Some(now + options.timeout);
                }
            }
            let message = match self.pong_deadline.or(self.next_ping) {
                Some(deadline) => match timeout_at(deadline, self.stream.next()).await {
                    Ok(Some(message)) => message,
                    Ok(None) => return Ok(WebSocketReceive::End),
                    Err(_elapsed) => continue,
                },
                None => match self.stream.next().await {
                    Some(message) => message,
                    None => return Ok(WebSocketReceive::End),
                },
            };
            match message {
                Ok(Message::Text(text)) => {
                    return Ok(WebSocketReceive::Text(text.as_str().to_owned()));
                }
                Ok(Message::Binary(bytes)) => return Ok(WebSocketReceive::Binary(bytes)),
                Ok(Message::Close(frame)) => {
                    let close = match frame {
                        Some(frame) => PeerClose {
                            code: u16::from(frame.code),
                            reason: frame.reason.as_str().to_owned(),
                        },
                        None => PeerClose {
                            code: EMPTY_CLOSE_CODE,
                            reason: String::new(),
                        },
                    };
                    return Ok(WebSocketReceive::Close(close));
                }
                Ok(Message::Pong(_)) => {
                    if let Some(options) = self.keepalive {
                        self.pong_deadline = None;
                        self.next_ping = Some(Instant::now() + options.interval);
                    }
                }
                // tungstenite answers a ping itself and a bare frame carries no message
                Ok(Message::Ping(_) | Message::Frame(_)) => {}
                Err(TungsteniteError::Protocol(ProtocolError::ResetWithoutClosingHandshake)) => {
                    return Ok(WebSocketReceive::End);
                }
                Err(error) => return Err(WebSocketClientError::from_tungstenite(error)),
            }
        }
    }
}
