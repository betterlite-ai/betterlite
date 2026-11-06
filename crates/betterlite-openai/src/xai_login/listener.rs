use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::callback::{
    CALLBACK_PATH, CallbackError, CallbackFields, CallbackRequest, decode_xai_callback,
};
use super::error::XaiLoginError;
use crate::xai_tokens::Clock;

pub const PREFERRED_PORT: u16 = 56_121;
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const MAX_REQUEST_BYTES: usize = 0x0001_0001;
const HTML: Option<&str> = Some("text/html; charset=utf-8");

pub struct CallbackSettings {
    pub preferred_port: u16,
    pub poll: Duration,
    pub timeout: Duration,
}

impl Default for CallbackSettings {
    fn default() -> Self {
        Self {
            preferred_port: PREFERRED_PORT,
            poll: POLL_INTERVAL,
            timeout: CALLBACK_TIMEOUT,
        }
    }
}

struct Page {
    status: u16,
    reason: &'static str,
    content_type: Option<&'static str>,
    body: &'static str,
}

const NOT_FOUND: Page = Page {
    status: 404,
    reason: "Not Found",
    content_type: None,
    body: "",
};
const BAD_REQUEST: Page = Page {
    status: 400,
    reason: "Bad Request",
    content_type: None,
    body: "",
};
const TOO_LONG: Page = Page {
    status: 414,
    reason: "URI Too Long",
    content_type: None,
    body: "",
};
const TOO_LARGE: Page = Page {
    status: 431,
    reason: "Request Header Fields Too Large",
    content_type: None,
    body: "",
};
const NOT_IMPLEMENTED: Page = Page {
    status: 501,
    reason: "Not Implemented",
    content_type: None,
    body: "",
};
const STATE_MISMATCH: Page = Page {
    status: 400,
    reason: "Bad Request",
    content_type: HTML,
    body: "<html><body><h1>xAI authorization state mismatch.</h1></body></html>",
};
const FAILED: Page = Page {
    status: 200,
    reason: "OK",
    content_type: HTML,
    body: "<html><body><h1>xAI authorization failed.</h1>You can close this tab.</body></html>",
};
const RECEIVED: Page = Page {
    status: 200,
    reason: "OK",
    content_type: HTML,
    body: "<html><body><h1>xAI authorization received.</h1>You can close this tab.</body></html>",
};

pub struct CallbackServer {
    pub(crate) listener: TcpListener,
    pub(crate) redirect_uri: String,
    pub(crate) expected_state: String,
    pub(crate) poll: Duration,
    pub(crate) timeout: Duration,
}

impl CallbackServer {
    pub async fn bind(
        settings: &CallbackSettings,
        expected_state: &str,
    ) -> Result<Self, XaiLoginError> {
        let mut failure = None;
        for candidate in [settings.preferred_port, 0] {
            match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, candidate))).await {
                Ok(listener) => {
                    let address = listener
                        .local_addr()
                        .map_err(|source| XaiLoginError::Bind { source })?;
                    let port = address.port();
                    return Ok(Self {
                        listener,
                        redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
                        expected_state: expected_state.to_owned(),
                        poll: settings.poll,
                        timeout: settings.timeout,
                    });
                }
                Err(source) => failure = Some(source),
            }
        }
        Err(XaiLoginError::Bind {
            source: failure.unwrap_or_else(|| io::Error::other("no callback port could be bound")),
        })
    }

    pub async fn wait<C>(self, clock: &C) -> Result<CallbackFields, XaiLoginError>
    where
        C: Clock,
    {
        let window = Decimal::from(self.timeout.as_secs())
            .checked_add(Decimal::new(i64::from(self.timeout.subsec_nanos()), 9))
            .ok_or(XaiLoginError::Deadline)?;
        let deadline = clock
            .now()
            .checked_add(window)
            .ok_or(XaiLoginError::Deadline)?;
        loop {
            let Some(slot) = wait_slot(clock.now(), deadline, self.poll) else {
                return Err(XaiLoginError::Timeout);
            };
            match timeout(slot, self.accept()).await {
                Ok(Ok(Some(fields))) => return Ok(fields),
                Ok(Ok(None)) => {}
                Ok(Err(source)) => return Err(XaiLoginError::Accept { source }),
                Err(_elapsed) => {}
            }
        }
    }

    async fn accept(&self) -> io::Result<Option<CallbackFields>> {
        let (mut stream, _peer) = self.listener.accept().await?;
        Ok(self.serve(&mut stream).await)
    }

    async fn serve(&self, stream: &mut TcpStream) -> Option<CallbackFields> {
        let head = match read_head(stream).await {
            Ok(Some(head)) => head,
            Ok(None) => {
                reply(stream, &TOO_LARGE).await;
                return None;
            }
            Err(error) => {
                tracing::debug!(%error, "Could not read the xAI OAuth callback request");
                return None;
            }
        };
        let request = match parse_request(&head) {
            Ok(request) => request,
            Err(page) => {
                reply(stream, page).await;
                return None;
            }
        };
        if request.path != CALLBACK_PATH {
            reply(stream, &NOT_FOUND).await;
            return None;
        }
        reply(stream, self.page_for(&request)).await;
        Some(request.fields)
    }

    fn page_for(&self, request: &CallbackRequest) -> &'static Page {
        if request.fields.state.as_deref() != Some(self.expected_state.as_str()) {
            return &STATE_MISMATCH;
        }
        if request.fields.error.is_some() {
            return &FAILED;
        }
        &RECEIVED
    }
}

fn parse_request(head: &[u8]) -> Result<CallbackRequest, &'static Page> {
    let Some((method, target)) = request_line(head) else {
        return Err(&BAD_REQUEST);
    };
    if method != b"GET" {
        return Err(&NOT_IMPLEMENTED);
    }
    match decode_xai_callback(target) {
        Ok(request) => Ok(request),
        Err(CallbackError::TooLong) => Err(&TOO_LONG),
        Err(CallbackError::Malformed) => Err(&BAD_REQUEST),
    }
}

// The poll waits one slot and the outer loop re-reads the clock, so the overall deadline bounds every iteration.
pub(crate) fn wait_slot(now: Decimal, deadline: Decimal, poll: Duration) -> Option<Duration> {
    let left = deadline.checked_sub(now)?;
    if left <= Decimal::ZERO {
        return None;
    }
    let nanos = left
        .checked_mul(Decimal::from(1_000_000_000_u64))
        .and_then(|value| value.to_u64())
        .unwrap_or(u64::MAX);
    Some(Duration::from_nanos(nanos).min(poll))
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(Some(buffer));
        }
        if buffer.len() >= MAX_REQUEST_BYTES {
            return Ok(None);
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(Some(buffer));
        }
        let bytes = chunk
            .get(..read)
            .ok_or_else(|| io::Error::other("the callback read exceeded its buffer"))?;
        buffer.extend_from_slice(bytes);
    }
}

fn request_line(head: &[u8]) -> Option<(&[u8], &[u8])> {
    let line = head.split(|byte| *byte == b'\n').next()?;
    let stripped = line.strip_suffix(b"\r").unwrap_or(line);
    let mut parts = stripped.split(|byte| *byte == b' ');
    let (Some(method), Some(target), Some(_version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some((method, target))
}

async fn reply(stream: &mut TcpStream, page: &Page) {
    if let Err(error) = write_page(stream, page).await {
        tracing::debug!(%error, "Could not write the xAI OAuth callback response");
    }
}

async fn write_page(stream: &mut TcpStream, page: &Page) -> io::Result<()> {
    let content_type = page
        .content_type
        .map_or_else(String::new, |value| format!("Content-Type: {value}\r\n"));
    let head = format!(
        "HTTP/1.1 {} {}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        page.status,
        page.reason,
        page.body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(page.body.as_bytes()).await?;
    stream.flush().await?;
    stream.shutdown().await
}
