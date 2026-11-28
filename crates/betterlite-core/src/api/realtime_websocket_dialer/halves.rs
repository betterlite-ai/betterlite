use async_trait::async_trait;
use betterlite_http::websocket_client::connection::{
    WebSocketReceive, WebSocketReceiver, WebSocketSender,
};
use betterlite_http::websocket_client::error::WebSocketClientError;
use betterlite_openai::realtime::message_cap::{
    MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON, oversized_message_close,
};

use crate::api::realtime_backend_relay::source::{RealtimeBackendSource, RelayReceiveError};
use crate::api::realtime_contracts::close::{CloseFrame, ConnectionClosed};
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};

// the close websockets' `async with websockets.connect(...)` sends on the way out of a health check
const NORMAL_CLOSE_CODE: u16 = 1000;

// the mixed close texts websockets' ConnectionClosed puts together from a sent and a received frame
const NO_CLOSE: &str = "no close frame received or sent";
const KEEPALIVE_CLOSE: &str =
    "sent 1011 (internal error) keepalive ping timeout; no close frame received";

// the sending half the session owns: text and binary frames out, a normal close at the end
pub(super) struct DialedSender {
    pub(super) sender: WebSocketSender,
}

#[async_trait]
impl RealtimeBackend for DialedSender {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        let outcome = match message {
            RealtimeMessage::Text(text) => self.sender.send_text(&text).await,
            RealtimeMessage::Binary(bytes) => self.sender.send_binary(&bytes).await,
        };
        outcome.map_err(websocket_error)
    }

    // B2457 splits the socket, so the relay reads the receiving half and never this one
    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: "the realtime backend websocket receiving half is the relay's source"
                .to_owned(),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        self.sender
            .close(NORMAL_CLOSE_CODE, "")
            .await
            .map_err(websocket_error)
    }
}

// the receiving half the relay polls: frames with the decode flag websockets' recv takes, and the close LiteLLM's relay reads
pub(super) struct DialedSource {
    pub(super) receiver: WebSocketReceiver,
    pub(super) sender: WebSocketSender,
    pub(super) max_size: usize,
}

#[async_trait]
impl RealtimeBackendSource for DialedSource {
    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        match self.receiver.recv().await {
            Ok(WebSocketReceive::Text(text)) => Ok(match decode {
                Some(false) => RealtimeMessage::Binary(text.into_bytes()),
                None | Some(true) => RealtimeMessage::Text(text),
            }),
            Ok(WebSocketReceive::Binary(bytes)) => match decode {
                Some(true) => match String::from_utf8(bytes.to_vec()) {
                    Ok(text) => Ok(RealtimeMessage::Text(text)),
                    Err(_error) => Err(RelayReceiveError::Failed(WebSocketError {
                        message: "the realtime backend sent a binary frame that is not UTF-8"
                            .to_owned(),
                    })),
                },
                None | Some(false) => Ok(RealtimeMessage::Binary(bytes.to_vec())),
            },
            Ok(WebSocketReceive::Close(close)) => {
                Err(RelayReceiveError::Closed(ConnectionClosed {
                    received: Some(CloseFrame {
                        code: close.code,
                        reason: close.reason.clone(),
                    }),
                    message: format!(
                        "received {}; no close frame sent",
                        close_text(close.code, &close.reason)
                    ),
                }))
            }
            Ok(WebSocketReceive::End) => Err(RelayReceiveError::Closed(ConnectionClosed {
                received: None,
                message: NO_CLOSE.to_owned(),
            })),
            Err(WebSocketClientError::Capacity { size, max_size }) => {
                let Some(close) = oversized_message_close(size, self.max_size) else {
                    return Err(RelayReceiveError::Failed(websocket_error(
                        WebSocketClientError::Capacity { size, max_size },
                    )));
                };
                let closed = RelayReceiveError::Closed(ConnectionClosed {
                    received: Some(CloseFrame {
                        code: close.code,
                        reason: close.reason,
                    }),
                    // the value SizeLimitedSource builds once a frame passes the client's own cap
                    message: format!(
                        "the realtime backend message exceeds the {}-byte limit",
                        self.max_size
                    ),
                });
                drop(
                    self.sender
                        .close(MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON)
                        .await,
                );
                Err(closed)
            }
            Err(WebSocketClientError::KeepaliveTimeout) => {
                Err(RelayReceiveError::Closed(ConnectionClosed {
                    received: None,
                    message: KEEPALIVE_CLOSE.to_owned(),
                }))
            }
            Err(error) => Err(RelayReceiveError::Failed(websocket_error(error))),
        }
    }
}

fn websocket_error(error: WebSocketClientError) -> WebSocketError {
    WebSocketError {
        message: error.to_string(),
    }
}

// websockets' frames.Close text: the code, its explanation and the reason
pub(super) fn close_text(code: u16, reason: &str) -> String {
    let explanation = match code {
        1000 => "OK",
        1001 => "going away",
        1002 => "protocol error",
        1003 => "unsupported data",
        1005 => "no status received [internal]",
        1006 => "abnormal closure [internal]",
        1007 => "invalid frame payload data",
        1008 => "policy violation",
        1009 => "message too big",
        1010 => "mandatory extension",
        1011 => "internal error",
        1012 => "service restart",
        1013 => "try again later",
        1014 => "bad gateway",
        1015 => "TLS handshake failure [internal]",
        3000..=3999 => "registered",
        4000..=4999 => "private use",
        _ => "unknown",
    };
    if reason.is_empty() {
        format!("{code} ({explanation})")
    } else {
        format!("{code} ({explanation}) {reason}")
    }
}
