use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::thread::park_timeout;
use std::time::Duration;
use std::time::Instant;

use thiserror::Error as ThisError;

use super::{
    CALLBACK_PATH, Clock,
    loopback::{LoopbackConnection, LoopbackListener},
    models::{CallbackCode, CallbackDenied, CallbackOutcome, PkceFailure},
    request::{CallbackQuery, RequestRead, parse_request},
};

pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, ThisError)]
pub enum CallbackError {
    #[error("loopback bind failed: {0}")]
    Bind(#[source] Error),
}

pub struct LoopbackServer<L: LoopbackListener> {
    listener: L,
    pub(super) address: SocketAddr,
    expected_state: String,
    connection_timeout: Duration,
}

impl<L: LoopbackListener> LoopbackServer<L> {
    pub fn with_listener(
        listener: L,
        expected_state: &str,
        connection_timeout: Duration,
    ) -> Result<Self, CallbackError> {
        let address = listener.local_addr().map_err(CallbackError::Bind)?;
        Ok(Self {
            listener,
            address,
            expected_state: expected_state.to_owned(),
            connection_timeout,
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
            match self.accept_request(clock, deadline) {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(error) if is_transient_accept_error(&error) => {}
                Err(error) => return Err(error),
            }
            park_timeout(Duration::from_millis(10));
        }
    }

    fn accept_request<C: Clock>(
        &self,
        clock: &C,
        deadline: Instant,
    ) -> Result<Option<CallbackOutcome>, PkceFailure> {
        let accepted = self
            .listener
            .accept()
            .map_err(|source| PkceFailure::LoopbackAccept { source })?;
        let Some(mut connection) = accepted else {
            return Ok(None);
        };
        let connection_deadline = clock
            .now()
            .checked_add(self.connection_timeout)
            .unwrap_or(deadline)
            .min(deadline);
        Ok(self.handle(&mut connection, clock, connection_deadline))
    }

    fn handle<C: Clock>(
        &self,
        connection: &mut L::Connection,
        clock: &C,
        deadline: Instant,
    ) -> Option<CallbackOutcome> {
        let query = read_callback_request(connection, clock, deadline)?;
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

// Re-parses the accumulated buffer after each bounded read; line and header limits cap the total buffer while the deadline caps each connection's lifetime.
fn read_callback_request<C, K>(
    connection: &mut C,
    clock: &K,
    deadline: Instant,
) -> Option<CallbackQuery>
where
    C: LoopbackConnection,
    K: Clock,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 65_536];
    loop {
        match parse_request(&buffer) {
            RequestRead::Complete(query) => return Some(query),
            RequestRead::UriTooLong => return respond_then(connection, 414, "URI too long.", None),
            RequestRead::HeadersTooLarge => {
                return respond_then(connection, 431, "Request header fields too large.", None);
            }
            RequestRead::Malformed => {
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
        let time_left = deadline.saturating_duration_since(clock.now());
        if time_left.is_zero() || connection.set_read_timeout(time_left).is_err() {
            return None;
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
