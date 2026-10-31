use secrecy::SecretString;
use std::str;
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
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    match request.parse(buffer) {
        Ok(httparse::Status::Partial) => RequestRead::Incomplete,
        Ok(httparse::Status::Complete(_)) => parse_parsed_request(&request),
        Err(httparse::Error::TooManyHeaders) => RequestRead::TooLarge,
        Err(httparse::Error::Version) if is_http_two_request(buffer) => {
            RequestRead::UnsupportedVersion
        }
        Err(_) => RequestRead::Malformed,
    }
}

fn is_http_two_request(buffer: &[u8]) -> bool {
    let Some(text) = str::from_utf8(buffer).ok() else {
        return false;
    };
    text.lines()
        .next()
        .is_some_and(|line| line.contains(" HTTP/2."))
}

fn parse_parsed_request(request: &httparse::Request<'_, '_>) -> RequestRead {
    let (Some(method), Some(path), Some(version)) = (request.method, request.path, request.version)
    else {
        return RequestRead::Malformed;
    };
    if version >= 2 {
        return RequestRead::UnsupportedVersion;
    }
    if method != "GET" {
        return RequestRead::UnsupportedMethod;
    }
    build_callback_query(path)
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

// Replicates Python's parse_qs(keep_blank_values=False) followed by taking the first remaining value for key: a blank value is dropped entirely, so it neither answers the query on its own nor hides a later, nonempty occurrence of the same key.
fn first_nonempty(url: &Url, key: &str) -> Option<String> {
    for (name, value) in url.query_pairs() {
        if name == key && !value.is_empty() {
            return Some(value.into_owned());
        }
    }
    None
}
