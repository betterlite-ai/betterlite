use super::error::RealtimeContractError;
use super::events::{ErrorEventType, RealtimeErrorDetail, RealtimeErrorEvent};
use super::transport::ClientWebSocket;
use thiserror::Error;
pub const CLOSE_REASON_MAX_BYTES: usize = 123;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseFrame {
    pub code: u16,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendClose {
    pub code: u16,
    pub reason: String,
}
impl BackendClose {
    #[must_use]
    pub fn message(&self) -> String {
        if self.reason.is_empty() {
            format!("upstream websocket closed with code {}", self.code)
        } else {
            format!(
                "upstream websocket closed with code {}: {}",
                self.code, self.reason
            )
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ConnectionClosed {
    pub received: Option<CloseFrame>,
    pub message: String,
}
#[must_use]
pub fn backend_close_from(error: &ConnectionClosed) -> BackendClose {
    error.received.as_ref().map_or_else(
        || BackendClose {
            code: 1006,
            reason: error.to_string(),
        },
        |frame| BackendClose {
            code: frame.code,
            reason: frame.reason.clone(),
        },
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientLoopExit {
    ClientDisconnected,
    BackendClosed,
}
pub fn realtime_error_event(
    message: &str,
    error_type: &str,
) -> Result<String, RealtimeContractError> {
    let event = RealtimeErrorEvent {
        event_type: ErrorEventType::Error,
        error: RealtimeErrorDetail {
            error_type: error_type.to_owned(),
            message: message.to_owned(),
        },
    };
    serde_json::to_string(&event).map_err(|source| RealtimeContractError::Encode { source })
}
#[must_use]
pub fn websocket_close_reason(message: &str, fallback: &str) -> String {
    if message.is_empty() {
        return fallback.to_owned();
    }
    let end = message.floor_char_boundary(CLOSE_REASON_MAX_BYTES);
    message.get(..end).unwrap_or_default().to_owned()
}
const EXTERNAL_CLOSE_CODES: [u16; 12] = [
    1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014,
];
#[must_use]
pub fn client_close_code(upstream_code: u16) -> u16 {
    let external = EXTERNAL_CLOSE_CODES.contains(&upstream_code);
    if external || (3000..5000).contains(&upstream_code) {
        return upstream_code;
    }
    1011
}
#[must_use]
pub const fn upstream_handshake_close_code(status_code: u16) -> u16 {
    match status_code {
        401 | 403 => 1008,
        429 => 1013,
        _ => 1011,
    }
}

pub async fn close_after_handshake_refusal<Socket>(
    socket: &mut Socket,
    status_code: u16,
) -> Result<(), RealtimeContractError>
where
    Socket: ClientWebSocket + ?Sized,
{
    let message = format!("Upstream realtime handshake rejected with HTTP {status_code}");
    let event = realtime_error_event(&message, "server_error")?;
    // a failed notice send must not skip the close below
    drop(socket.send_text(&event).await);
    socket
        .close(
            upstream_handshake_close_code(status_code),
            Some(&websocket_close_reason(
                &message,
                "Upstream handshake rejected",
            )),
        )
        .await
        .map_err(RealtimeContractError::Socket)
}
