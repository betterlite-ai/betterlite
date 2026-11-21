use thiserror::Error;

use crate::api::realtime_contracts::close::{
    client_close_code, realtime_error_event, websocket_close_reason,
};
use crate::api::realtime_contracts::error::{RealtimeContractError, WebSocketError};
use crate::api::realtime_contracts::transport::ClientWebSocket;

#[derive(Debug, Error)]
enum CloseDeliveryError {
    #[error(transparent)]
    Contract(#[from] RealtimeContractError),
    #[error(transparent)]
    Socket(#[from] WebSocketError),
}

// LiteLLM sends the redacted error event for any code but 1000, then closes with the legal code and a safe reason
async fn deliver(
    socket: &mut dyn ClientWebSocket,
    code: u16,
    message: &str,
    reason: &str,
) -> Result<(), CloseDeliveryError> {
    if code != 1000 {
        let event = realtime_error_event(message, "server_error")?;
        socket.send_text(&event).await?;
    }
    socket
        .close(
            client_close_code(code),
            Some(&websocket_close_reason(reason, message)),
        )
        .await?;
    Ok(())
}

// One failed attempt ends this method: a failed notice send skips the close, exactly as LiteLLM's single try does
pub(crate) async fn relay(
    socket: &mut Option<Box<dyn ClientWebSocket>>,
    code: u16,
    message: &str,
    reason: &str,
) {
    let Some(connection) = socket.as_deref_mut() else {
        tracing::debug!(
            "Could not relay the upstream close to the client: the realtime client websocket is not connected"
        );
        return;
    };
    if let Err(error) = deliver(connection, code, message, reason).await {
        tracing::debug!("Could not relay the upstream close to the client: {error}");
    }
}
