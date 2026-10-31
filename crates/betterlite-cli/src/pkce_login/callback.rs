use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::thread::park_timeout;
use std::time::Duration;

use thiserror::Error as ThisError;

use super::{
    CALLBACK_PATH, Clock,
    loopback::{LoopbackConnection, LoopbackListener, TcpLoopbackListener},
    models::{CallbackCode, CallbackDenied, CallbackOutcome, PkceFailure},
    request::{CallbackQuery, RequestRead, parse_request},
};

#[derive(Debug, ThisError)]
pub enum CallbackError {
    #[error("loopback bind failed: {0}")]
    Bind(#[source] Error),
}

pub struct LoopbackServer<L: LoopbackListener> {
    listener: L,
    pub(super) address: SocketAddr,
    expected_state: String,
}

impl LoopbackServer<TcpLoopbackListener> {
    pub fn bind(expected_state: &str) -> Result<Self, CallbackError> {
        let listener = TcpLoopbackListener::bind().map_err(CallbackError::Bind)?;
        Self::with_listener(listener, expected_state)
    }
}

impl<L: LoopbackListener> LoopbackServer<L> {
    pub(super) fn with_listener(listener: L, expected_state: &str) -> Result<Self, CallbackError> {
        let address = listener.local_addr().map_err(CallbackError::Bind)?;
        Ok(Self {
            listener,
            address,
            expected_state: expected_state.to_owned(),
        })
    }

    #[must_use]
    pub fn redirect_uri(&self) -> String {
        let port = self.address.port();
        format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
    }

    pub fn wait<C>(&mut self, timeout: Duration, clock: &C) -> Result<CallbackOutcome, PkceFailure>
    where
        C: Clock,
    {
        let deadline = clock
            .now()
            .checked_add(timeout)
            .ok_or(PkceFailure::CallbackTimeoutOverflow)?;
        loop {
            if clock.now() >= deadline {
                return Err(PkceFailure::CallbackTimeout);
            }
            match self.accept_request() {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(error) if is_transient_accept_error(&error) => {}
                Err(error) => return Err(error),
            }
            park_timeout(Duration::from_millis(10));
        }
    }

    fn accept_request(&self) -> Result<Option<CallbackOutcome>, PkceFailure> {
        let accepted = self
            .listener
            .accept()
            .map_err(|source| PkceFailure::LoopbackAccept { source })?;
        let Some(mut connection) = accepted else {
            return Ok(None);
        };
        Ok(self.handle(&mut connection))
    }

    fn handle(&self, connection: &mut L::Connection) -> Option<CallbackOutcome> {
        let query = read_callback_request(connection)?;
        if query.path != CALLBACK_PATH {
            return respond_then(connection, 404, "Not found.", None);
        }
        if query.state.as_deref() != Some(self.expected_state.as_str()) {
            return respond_then(
                connection,
                400,
                "This response does not belong to the pending sign-in; still waiting.",
                None,
            );
        }
        if let Some(error) = query.error {
            let outcome = CallbackOutcome::Denied(CallbackDenied {
                error,
                description: query.description,
            });
            return respond_then(
                connection,
                200,
                "Sign-in was not approved. You can close this window.",
                Some(outcome),
            );
        }
        let Some(code) = query.code else {
            return respond_then(
                connection,
                400,
                "The sign-in response carried no authorization code; still waiting.",
                None,
            );
        };
        respond_then(
            connection,
            200,
            "Signed in. You can close this window.",
            Some(CallbackOutcome::Code(CallbackCode { code })),
        )
    }
}

fn is_transient_accept_error(error: &PkceFailure) -> bool {
    let PkceFailure::LoopbackAccept { source } = error else {
        return false;
    };
    matches!(
        source.kind(),
        ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::Interrupted
            | ErrorKind::WouldBlock
    )
}

// Re-parses the accumulated buffer after every read, so a request split across several TCP reads parses like one that arrives whole, and drops the connection silently on any error so the wait loop just keeps polling.
fn read_callback_request<C>(connection: &mut C) -> Option<CallbackQuery>
where
    C: LoopbackConnection,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match parse_request(&buffer) {
            RequestRead::Complete(query) => return Some(query),
            RequestRead::TooLarge | RequestRead::Malformed => {
                return respond_then(connection, 400, "Invalid callback.", None);
            }
            RequestRead::UnsupportedMethod => {
                return respond_then(connection, 501, "Unsupported method.", None);
            }
            RequestRead::UnsupportedVersion => {
                return respond_then(connection, 505, "Unsupported HTTP version.", None);
            }
            RequestRead::Incomplete => {}
        }
        match connection.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(size) => {
                let bytes = chunk.get(..size)?;
                buffer.extend_from_slice(bytes);
            }
        }
    }
}

// Sends the response and hands back `result`, so a handler's one line names what it reports without repeating the send.
fn respond_then<C, T>(connection: &mut C, status: u16, body: &str, result: T) -> T
where
    C: LoopbackConnection,
{
    drop(respond(connection, status, body));
    result
}

fn respond<C>(connection: &mut C, status: u16, body: &str) -> Result<(), PkceFailure>
where
    C: LoopbackConnection,
{
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    connection
        .write_all(response.as_bytes())
        .map_err(|source| PkceFailure::LoopbackWrite { source })
}
