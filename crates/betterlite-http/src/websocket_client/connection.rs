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
        self.write_message(Message::Text(Utf8Bytes::from(text)))
            .await
    }

    pub async fn send_binary(&self, bytes: &[u8]) -> Result<(), WebSocketClientError> {
        self.write_message(Message::Binary(Bytes::copy_from_slice(bytes)))
            .await
    }

    pub async fn close(&self, code: u16, reason: &str) -> Result<(), WebSocketClientError> {
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: Utf8Bytes::from(reason),
        };
        self.write_message(Message::Close(Some(frame))).await
    }

    async fn write_message(&self, message: Message) -> Result<(), WebSocketClientError> {
        let mut sink = self.sink.lock().await;
        sink.send(message)
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
            let next = self.receive().await?;
            if let Some(message) = next {
                return Ok(message);
            }
        }
    }

    // websockets' keepalive task: a ping every interval, and a socket that misses a pong fails with 1011
    async fn keepalive_step(&mut self) -> Result<(), WebSocketClientError> {
        let Some(options) = self.keepalive else {
            return Ok(());
        };
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
            let mut sink = self.sender.sink.lock().await;
            sink.send(Message::Ping(Bytes::new()))
                .await
                .map_err(WebSocketClientError::from_tungstenite)?;
            self.pong_deadline = Some(now + options.timeout);
        }
        Ok(())
    }

    // one message from the peer, waiting up to the keepalive deadline and skipping frames that carry none
    async fn receive(&mut self) -> Result<Option<WebSocketReceive>, WebSocketClientError> {
        self.keepalive_step().await?;
        let waited = match self.pong_deadline.or(self.next_ping) {
            Some(deadline) => timeout_at(deadline, self.stream.next()).await,
            None => Ok(self.stream.next().await),
        };
        let message = match waited {
            Ok(Some(message)) => message,
            Ok(None) => return Ok(Some(WebSocketReceive::End)),
            Err(_elapsed) => return Ok(None),
        };
        match message {
            Ok(Message::Text(text)) => Ok(Some(WebSocketReceive::Text(text.as_str().to_owned()))),
            Ok(Message::Binary(bytes)) => Ok(Some(WebSocketReceive::Binary(bytes))),
            Ok(Message::Close(frame)) => Ok(Some(WebSocketReceive::Close(peer_close(frame)))),
            Ok(Message::Pong(_)) => {
                self.pong_received();
                Ok(None)
            }
            // tungstenite answers a ping itself and a bare frame carries no message
            Ok(Message::Ping(_) | Message::Frame(_)) => Ok(None),
            Err(TungsteniteError::Protocol(ProtocolError::ResetWithoutClosingHandshake)) => {
                Ok(Some(WebSocketReceive::End))
            }
            Err(error) => Err(WebSocketClientError::from_tungstenite(error)),
        }
    }

    fn pong_received(&mut self) {
        let Some(options) = self.keepalive else {
            return;
        };
        self.pong_deadline = None;
        self.next_ping = Some(Instant::now() + options.interval);
    }
}

// websockets' Close, with 1005 and an empty reason when the peer sent no code
fn peer_close(frame: Option<CloseFrame>) -> PeerClose {
    let Some(close) = frame else {
        return PeerClose {
            code: EMPTY_CLOSE_CODE,
            reason: String::new(),
        };
    };
    PeerClose {
        code: u16::from(close.code),
        reason: close.reason.as_str().to_owned(),
    }
}
