use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::callback::{CALLBACK_PATH, CallbackError, CallbackRequest, decode_xai_callback};
use super::error::XaiLoginError;
use crate::xai_tokens::Clock;

pub const PREFERRED_PORT: u16 = 56_121;
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const MAX_HEAD_BYTES: u64 = 0x0001_0001;
const MISMATCH_BODY: &str = "<html><body><h1>xAI authorization state mismatch.</h1></body></html>";
const FAILED_BODY: &str =
    "<html><body><h1>xAI authorization failed.</h1>You can close this tab.</body></html>";
const RECEIVED_BODY: &str =
    "<html><body><h1>xAI authorization received.</h1>You can close this tab.</body></html>";

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

enum Head {
    Partial,
    Ready(Result<CallbackRequest, (u16, &'static str)>),
}

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
        let preferred = SocketAddr::from((Ipv4Addr::LOCALHOST, settings.preferred_port));
        let listener = match TcpListener::bind(preferred).await {
            Ok(listener) => listener,
            Err(_preferred_taken) => {
                let ephemeral = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
                TcpListener::bind(ephemeral)
                    .await
                    .map_err(|source| XaiLoginError::Listener { source })?
            }
        };
        let address = listener
            .local_addr()
            .map_err(|source| XaiLoginError::Listener { source })?;
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{}{CALLBACK_PATH}", address.port()),
            expected_state: expected_state.to_owned(),
            poll: settings.poll,
            timeout: settings.timeout,
        })
    }

    pub async fn wait<C>(self, clock: &C) -> Result<CallbackRequest, XaiLoginError>
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
            let mut stream = match timeout(slot, self.listener.accept()).await {
                Ok(Ok((stream, _peer))) => stream,
                Ok(Err(source)) => return Err(XaiLoginError::Listener { source }),
                Err(_elapsed) => continue,
            };
            if let Some(request) = self.serve(&mut stream).await {
                return Ok(request);
            }
        }
    }

    async fn serve(&self, stream: &mut TcpStream) -> Option<CallbackRequest> {
        let head = match timeout(self.poll, read_head(stream)).await {
            Ok(Ok(head)) => head,
            Ok(Err(error)) => {
                tracing::debug!(%error, "Could not read the xAI OAuth callback request");
                return None;
            }
            Err(_elapsed) => {
                tracing::debug!("The xAI OAuth callback connection sent no request in time");
                return None;
            }
        };
        let request = match decode_head(&head) {
            Head::Ready(Ok(request)) => request,
            Head::Ready(Err((status, reason))) => {
                reply(stream, status, reason, "").await;
                return None;
            }
            Head::Partial => {
                reply(stream, 400, "Bad Request", "").await;
                return None;
            }
        };
        if request.path != CALLBACK_PATH {
            reply(stream, 404, "Not Found", "").await;
            return None;
        }
        let (status, reason, body) = self.page(&request);
        reply(stream, status, reason, body).await;
        Some(request)
    }

    fn page(&self, request: &CallbackRequest) -> (u16, &'static str, &'static str) {
        if request.state.as_deref() != Some(self.expected_state.as_str()) {
            return (400, "Bad Request", MISMATCH_BODY);
        }
        if request.error.is_some() {
            return (200, "OK", FAILED_BODY);
        }
        (200, "OK", RECEIVED_BODY)
    }
}

fn decode_head(head: &[u8]) -> Head {
    let mut headers = [httparse::EMPTY_HEADER; 99];
    let mut request = httparse::Request::new(&mut headers);
    match request.parse(head) {
        Ok(httparse::Status::Partial) => Head::Partial,
        Ok(httparse::Status::Complete(_consumed)) => Head::Ready(parse_target(&request)),
        Err(httparse::Error::TooManyHeaders) => {
            Head::Ready(Err((431, "Request Header Fields Too Large")))
        }
        Err(_malformed) => Head::Ready(Err((400, "Bad Request"))),
    }
}

fn parse_target(head: &httparse::Request<'_, '_>) -> Result<CallbackRequest, (u16, &'static str)> {
    let (Some(method), Some(target)) = (head.method, head.path) else {
        return Err((400, "Bad Request"));
    };
    if method != "GET" {
        return Err((501, "Not Implemented"));
    }
    match decode_xai_callback(target.as_bytes()) {
        Ok(request) => Ok(request),
        Err(CallbackError::TooLong) => Err((414, "URI Too Long")),
        Err(CallbackError::Malformed) => Err((400, "Bad Request")),
    }
}

// The poll waits one slot and the outer loop re-reads the clock, so the overall deadline bounds every iteration.
pub(crate) fn wait_slot(now: Decimal, deadline: Decimal, poll: Duration) -> Option<Duration> {
    let left = deadline.checked_sub(now)?;
    if left <= Decimal::ZERO {
        return None;
    }
    let slot = left
        .checked_mul(Decimal::from(1_000_000_000_u64))
        .and_then(|value| value.to_u64())
        .map_or(poll, Duration::from_nanos);
    Some(slot.min(poll))
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let mut limited = stream.take(MAX_HEAD_BYTES);
    loop {
        let read = limited.read(&mut chunk).await?;
        if read == 0 {
            return Ok(buffer);
        }
        let bytes = chunk
            .get(..read)
            .ok_or_else(|| io::Error::other("callback read overran"))?;
        buffer.extend_from_slice(bytes);
        if !matches!(decode_head(&buffer), Head::Partial) {
            return Ok(buffer);
        }
    }
}

async fn reply(stream: &mut TcpStream, status: u16, reason: &'static str, body: &'static str) {
    let content_type = if body.is_empty() {
        ""
    } else {
        "Content-Type: text/html; charset=utf-8\r\n"
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let written = async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        stream.flush().await?;
        stream.shutdown().await
    };
    if let Err(error) = written.await {
        tracing::debug!(%error, "Could not write the xAI OAuth callback response");
    }
}
