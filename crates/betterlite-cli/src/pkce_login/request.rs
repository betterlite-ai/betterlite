use std::str;

use url::Url;

// Bounds how many bytes a connection may send before its request is given up on, so a drip-fed or endless request cannot grow the buffer without limit.
pub const MAX_REQUEST_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackQuery {
    pub(super) path: String,
    pub(super) state: Option<String>,
    pub(super) error: Option<String>,
    pub(super) description: Option<String>,
    pub(super) code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestRead {
    // The buffer holds no complete request line and headers yet; read more and try again.
    Incomplete,
    // The buffer grew past MAX_REQUEST_BYTES without completing a request.
    TooLarge,
    // The request line or headers could not be parsed as HTTP/1.x.
    Malformed,
    // The request line named a method other than GET.
    UnsupportedMethod,
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
    let mut tokens = request_line.split_whitespace();
    let (Some(method), Some(target), Some(_version)) =
        (tokens.next(), tokens.next(), tokens.next())
    else {
        return RequestRead::Malformed;
    };
    if tokens.next().is_some() {
        return RequestRead::Malformed;
    }
    if method != "GET" {
        return RequestRead::UnsupportedMethod;
    }
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return RequestRead::Malformed;
    };
    RequestRead::Complete(CallbackQuery {
        path: url.path().to_owned(),
        state: first_nonempty(&url, "state"),
        error: first_nonempty(&url, "error"),
        description: first_nonempty(&url, "error_description"),
        code: first_nonempty(&url, "code"),
    })
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
