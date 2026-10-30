use std::io::{Error, ErrorKind, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str::from_utf8;
use std::thread::park_timeout;
use std::time::Duration;

use secrecy::SecretString;
use thiserror::Error;
use url::Url;

use super::{
    CALLBACK_PATH, Clock,
    models::{CallbackCode, CallbackDenied, CallbackOutcome, PkceFailure},
};

#[derive(Debug, Error)]
pub enum CallbackError {
    #[error("loopback bind failed: {0}")]
    Bind(#[source] Error),
}

pub struct LoopbackServer {
    listener: TcpListener,
    pub(super) address: SocketAddr,
    expected_state: String,
}

impl LoopbackServer {
    pub fn bind(expected_state: &str) -> Result<Self, CallbackError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(CallbackError::Bind)?;
        listener
            .set_nonblocking(true)
            .map_err(CallbackError::Bind)?;
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
            if let Some(outcome) = self.accept_request()? {
                return Ok(outcome);
            }
            park_timeout(Duration::from_millis(10));
        }
    }

    fn accept_request(&self) -> Result<Option<CallbackOutcome>, PkceFailure> {
        let accepted = match self.listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(error) => {
                return Err(PkceFailure::LoopbackAccept { source: error });
            }
        };
        let (mut stream, _) = accepted;
        if stream.set_nonblocking(false).is_err()
            || stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .is_err()
        {
            return Ok(None);
        }
        Ok(self.handle(&mut stream))
    }

    fn handle(&self, stream: &mut TcpStream) -> Option<CallbackOutcome> {
        let parsed = callback_url(stream)?;
        if parsed.path() != CALLBACK_PATH {
            drop(respond(stream, 404, "Not found."));
            return None;
        }
        let state = parsed
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned());
        if state.as_deref() != Some(self.expected_state.as_str()) {
            drop(respond(
                stream,
                400,
                "This response does not belong to the pending sign-in; still waiting.",
            ));
            return None;
        }
        settle_callback(stream, &parsed)
    }
}

fn callback_url(stream: &mut TcpStream) -> Option<Url> {
    let mut bytes = [0_u8; 8192];
    let Ok(size) = stream.read(&mut bytes) else {
        return None;
    };
    let Some(request_bytes) = bytes.get(..size) else {
        drop(respond(stream, 400, "Invalid callback."));
        return None;
    };
    let Ok(request) = from_utf8(request_bytes) else {
        drop(respond(stream, 400, "Invalid callback."));
        return None;
    };
    let Some(path) = request.split_whitespace().nth(1) else {
        drop(respond(stream, 400, "Invalid callback."));
        return None;
    };
    let Ok(parsed) = Url::parse(&format!("http://localhost{path}")) else {
        drop(respond(stream, 400, "Invalid callback."));
        return None;
    };
    Some(parsed)
}

fn settle_callback(stream: &mut TcpStream, parsed: &Url) -> Option<CallbackOutcome> {
    let callback_error = parsed
        .query_pairs()
        .find(|(name, _)| name == "error")
        .map(|(_, value)| value.into_owned());
    if let Some(error) = callback_error {
        let description = parsed
            .query_pairs()
            .find(|(name, _)| name == "error_description")
            .map(|(_, value)| value.into_owned());
        let outcome = CallbackOutcome::Denied(CallbackDenied { error, description });
        drop(respond(
            stream,
            200,
            "Sign-in was not approved. You can close this window.",
        ));
        return Some(outcome);
    }
    let Some(code) = parsed
        .query_pairs()
        .find(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
    else {
        drop(respond(
            stream,
            400,
            "The sign-in response carried no authorization code; still waiting.",
        ));
        return None;
    };
    drop(respond(
        stream,
        200,
        "Signed in. You can close this window.",
    ));
    Some(CallbackOutcome::Code(CallbackCode {
        code: SecretString::from(code),
    }))
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), PkceFailure> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|source| PkceFailure::LoopbackWrite { source })
}
