use betterlite_redact::secret::Redactor;

use super::error::RealtimeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeClose {
    pub code: u16,
    pub reason: String,
}

#[derive(Debug, Clone, Copy)]
pub enum RealtimeFailure<'failure> {
    Handshake {
        status_code: u16,
    },
    Other {
        message: &'failure str,
    },
    CloseFailed {
        message: &'failure str,
        close_error: &'failure str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAction {
    RefuseHandshake { status_code: u16 },
    Close(RealtimeClose),
    IgnoreCloseFailure { attempted: RealtimeClose },
}

pub fn client_action(
    failure: RealtimeFailure<'_>,
    redactor: &Redactor,
) -> Result<ClientAction, RealtimeError> {
    match failure {
        RealtimeFailure::Handshake { status_code } => {
            Ok(ClientAction::RefuseHandshake { status_code })
        }
        RealtimeFailure::Other { message } => {
            Ok(ClientAction::Close(internal_error_close(message, redactor)))
        }
        RealtimeFailure::CloseFailed {
            message,
            close_error,
        } => {
            let attempted = internal_error_close(message, redactor);
            if close_error.contains("already completed") || close_error.contains("websocket.close")
            {
                return Ok(ClientAction::IgnoreCloseFailure { attempted });
            }
            Err(RealtimeError::UnexpectedCloseFailure {
                close_error: close_error.to_owned(),
            })
        }
    }
}

fn internal_error_close(message: &str, redactor: &Redactor) -> RealtimeClose {
    RealtimeClose {
        code: 1011,
        reason: redactor.redact_string(&format!("Internal server error: {message}")),
    }
}
