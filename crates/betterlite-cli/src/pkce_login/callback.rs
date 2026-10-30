use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use url::Url;

use super::{CALLBACK_PATH, CallbackCode, CallbackDenied, CallbackOutcome, PkceFailure};

#[derive(Debug)]
pub enum CallbackError {
    Bind(std::io::Error),
}

pub struct LoopbackServer {
    listener: TcpListener,
    address: SocketAddr,
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

    pub fn local_addr(&self) -> SocketAddr {
        let address = self.address;
        address
    }

    pub fn redirect_uri(&self) -> String {
        let port = self.local_addr().port();
        format!("http://127.0.0.1:{}{CALLBACK_PATH}", port)
    }

    pub fn wait(&mut self, timeout: Duration) -> Result<CallbackOutcome, PkceFailure> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| PkceFailure::new("callback timeout overflow"))?;
        loop {
            if Instant::now() >= deadline {
                return Err(PkceFailure::new(
                    "timed out waiting for the browser sign-in to finish",
                ));
            }
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .map_err(|error| PkceFailure::new(error.to_string()))?;
                    if let Some(outcome) = self.handle(&mut stream)? {
                        return Ok(outcome);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(PkceFailure::new(format!("loopback accept failed: {error}")));
                }
            }
        }
    }

    fn handle(&self, stream: &mut TcpStream) -> Result<Option<CallbackOutcome>, PkceFailure> {
        let mut bytes = [0_u8; 8192];
        let size = stream
            .read(&mut bytes)
            .map_err(|error| PkceFailure::new(format!("loopback read failed: {error}")))?;
        let request = std::str::from_utf8(&bytes[..size])
            .map_err(|error| PkceFailure::new(format!("invalid callback request: {error}")))?;
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        let parsed = Url::parse(&format!("http://localhost{path}"));
        let parsed = match parsed {
            Ok(value) => value,
            Err(_) => {
                respond(stream, 400, "Invalid callback.")?;
                return Ok(None);
            }
        };
        if parsed.path() != CALLBACK_PATH {
            respond(stream, 404, "Not found.")?;
            return Ok(None);
        }
        let state = query_value(&parsed, "state");
        if state.as_deref() != Some(self.expected_state.as_str()) {
            respond(
                stream,
                400,
                "This response does not belong to the pending sign-in; still waiting.",
            )?;
            return Ok(None);
        }
        if let Some(error) = query_value(&parsed, "error") {
            let outcome = CallbackOutcome::Denied(CallbackDenied {
                error,
                description: query_value(&parsed, "error_description"),
            });
            respond(
                stream,
                200,
                "Sign-in was not approved. You can close this window.",
            )?;
            return Ok(Some(outcome));
        }
        let Some(code) = query_value(&parsed, "code") else {
            respond(
                stream,
                400,
                "The sign-in response carried no authorization code; still waiting.",
            )?;
            return Ok(None);
        };
        respond(stream, 200, "Signed in. You can close this window.")?;
        Ok(Some(CallbackOutcome::Code(CallbackCode { code })))
    }
}

fn query_value(url: &Url, key: &str) -> Option<String> {
    let pairs = url.query_pairs();
    pairs
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), PkceFailure> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| PkceFailure::new(format!("loopback write failed: {error}")))
}
