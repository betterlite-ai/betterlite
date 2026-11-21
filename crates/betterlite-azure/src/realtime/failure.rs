use betterlite_openai::realtime::failure::RealtimeClose;

pub const SESSION_ERROR_TYPE: &str = "server_error";
pub const SESSION_ERROR_MESSAGE: &str = "Internal server error";
pub const SESSION_CLOSE_CODE: u16 = 1011;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeFailure {
    Handshake { status_code: u16 },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAction {
    RefuseHandshake {
        status_code: u16,
    },
    SendErrorAndClose {
        error_type: &'static str,
        message: &'static str,
        close: RealtimeClose,
    },
}

#[must_use]
pub fn client_action(failure: RealtimeFailure) -> ClientAction {
    match failure {
        RealtimeFailure::Handshake { status_code } => ClientAction::RefuseHandshake { status_code },
        RealtimeFailure::Other => ClientAction::SendErrorAndClose {
            error_type: SESSION_ERROR_TYPE,
            message: SESSION_ERROR_MESSAGE,
            close: RealtimeClose {
                code: SESSION_CLOSE_CODE,
                reason: String::from(SESSION_ERROR_MESSAGE),
            },
        },
    }
}
