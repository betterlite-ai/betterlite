use std::str;

use secrecy::SecretString;
use url::Url;

// Bounds how many bytes a connection may send before its request is given up on, so a drip-fed or endless request cannot grow the buffer without limit.
pub const MAX_REQUEST_BYTES: usize = 8192;

// secrecy::SecretString already carries a redacting Debug impl ("SecretBox<str>([REDACTED])"), so deriving Debug here is safe: it never prints `code`.
#[derive(Debug, Clone)]
pub struct CallbackQuery {
    pub(super) path: String,
    pub(super) state: Option<String>,
    pub(super) error: Option<String>,
    pub(super) description: Option<String>,
    pub(super) code: Option<SecretString>,
}

// CallbackQuery holds a SecretString, which has no PartialEq/Eq; callers compare individual fields or use `matches!` instead of `assert_eq!` on the whole value.
#[derive(Debug, Clone)]
pub enum RequestRead {
    // The buffer holds no complete request line and headers yet; read more and try again.
    Incomplete,
    // The buffer grew past MAX_REQUEST_BYTES without completing a request.
    TooLarge,
    // The request line or headers could not be parsed as HTTP/1.x.
    Malformed,
    // The request line named a method other than GET.
    UnsupportedMethod,
    // The request line named an HTTP version of 2.0 or newer.
    UnsupportedVersion,
    Complete(CallbackQuery),
}

// Parses as much of an HTTP/1.x request as buffer holds. Callers feed it the bytes read so far after every read; it reports Incomplete until the request line and headers are fully buffered, so a request split across several TCP reads parses the same as one that arrives whole.
#[must_use]
pub fn parse_request(buffer: &[u8]) -> RequestRead {
    if buffer.len() > MAX_REQUEST_BYTES {
        return RequestRead::TooLarge;
    }
    let Some(headers_end) = find_end_of_headers(buffer) else {
        return RequestRead::Incomplete;
    };
    let Some(head_bytes) = buffer.get(..headers_end) else {
        return RequestRead::Malformed;
    };
    let Ok(head) = str::from_utf8(head_bytes) else {
        return RequestRead::Malformed;
    };
    let Some(request_line) = head.split("\r\n").next() else {
        return RequestRead::Malformed;
    };
    match parse_request_line(request_line) {
        Ok(target) => build_callback_query(target),
        Err(outcome) => outcome,
    }
}

// Validates the method and HTTP version on the request line and hands back the target to parse as a URL, so parse_request itself stays short.
fn parse_request_line(line: &str) -> Result<&str, RequestRead> {
    let mut tokens = line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (tokens.next(), tokens.next(), tokens.next())
    else {
        return Err(RequestRead::Malformed);
    };
    if tokens.next().is_some() {
        return Err(RequestRead::Malformed);
    }
    let Some((major, _minor)) = parse_http_version(version) else {
        return Err(RequestRead::Malformed);
    };
    if major >= 2 {
        return Err(RequestRead::UnsupportedVersion);
    }
    if method != "GET" {
        return Err(RequestRead::UnsupportedMethod);
    }
    Ok(target)
}

fn build_callback_query(target: &str) -> RequestRead {
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return RequestRead::Malformed;
    };
    RequestRead::Complete(CallbackQuery {
        path: url.path().to_owned(),
        state: first_nonempty(&url, "state"),
        error: first_nonempty(&url, "error"),
        description: first_nonempty(&url, "error_description"),
        code: first_nonempty(&url, "code").map(SecretString::from),
    })
}

// Mirrors Python's BaseHTTPRequestHandler.parse_request: the version token must read "HTTP/<digits>.<digits>", each component no more than 10 digits, so "nonsense" or "HTTP/1.1.1" are rejected rather than silently ignored.
fn parse_http_version(version: &str) -> Option<(u32, u32)> {
    let numbers = version.strip_prefix("HTTP/")?;
    let mut parts = numbers.split('.');
    let major_text = parts.next()?;
    let minor_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let major = parse_version_component(major_text)?;
    let minor = parse_version_component(minor_text)?;
    Some((major, minor))
}

fn parse_version_component(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

fn find_end_of_headers(buffer: &[u8]) -> Option<usize> {
    let position = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    position.checked_add(4)
}

// Replicates Python's parse_qs(keep_blank_values=False) followed by taking the first remaining value for key: a blank value is dropped entirely, so it neither answers the query on its own nor hides a later, nonempty occurrence of the same key.
fn first_nonempty(url: &Url, key: &str) -> Option<String> {
    for (name, value) in url.query_pairs() {
        if name == key && !value.is_empty() {
            return Some(value.into_owned());
        }
    }
    None
}
