use std::collections::BTreeMap;
use std::io::{Error, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::str::{self, Utf8Error};
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

use crate::case::Request;

pub(crate) const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

pub(crate) struct Client;

#[derive(Debug, Error)]
pub(crate) enum HttpError {
    #[error("invalid HTTP method {method}")]
    Method { method: String },
    #[error("invalid local gateway URL {url}")]
    Url { url: String },
    #[error("cannot connect to {address}: {source}")]
    Connect { address: String, source: Error },
    #[error("HTTP I/O failed: {0}")]
    Io(#[source] Error),
    #[error("HTTP response exceeded 16 MiB")]
    TooLarge,
    #[error("invalid HTTP response: {0}")]
    Response(String),
    #[error("response body is not valid UTF-8: {0}")]
    Text(#[source] Utf8Error),
    #[error("response body event is not valid JSON: {0}")]
    Body(#[source] serde_json::Error),
}

pub(crate) fn send(client: &Client, base: &str, request: &Request) -> Result<Response, HttpError> {
    let _ = client;
    let method = validate_method(&request.method)?;
    let (address, authority) = parse_base(base)?;
    let body = if request.body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&request.body).map_err(|error| HttpError::Response(error.to_string()))?
    };
    let mut headers = request.headers.clone();
    headers.insert("content-length".to_owned(), body.len().to_string());
    headers.insert("connection".to_owned(), "close".to_owned());
    let mut stream = connect(address)?;
    write_request(
        &mut stream,
        method,
        &request.path,
        &authority,
        &headers,
        &body,
    )?;
    read_response(&mut stream)
}

pub(crate) fn ready(client: &Client, base: &str) -> Result<bool, HttpError> {
    let request = Request {
        method: "GET".to_owned(),
        path: "/health/liveliness".to_owned(),
        headers: BTreeMap::new(),
        body: Value::Null,
    };
    Ok(is_ready_status(send(client, base, &request)?.status))
}

pub(crate) fn is_ready_status(status: u16) -> bool {
    let ready = status == 200;
    ready
}

pub(crate) fn validate_method(method: &str) -> Result<&str, HttpError> {
    if method.is_empty() || method.chars().any(|character| character.is_whitespace()) {
        return Err(HttpError::Method {
            method: method.to_owned(),
        });
    }
    Ok(method)
}

pub(crate) fn parse_base(base: &str) -> Result<(SocketAddr, String), HttpError> {
    let authority = base.strip_prefix("http://").ok_or_else(|| HttpError::Url {
        url: base.to_owned(),
    })?;
    let address = authority
        .parse::<SocketAddr>()
        .map_err(|_| HttpError::Url {
            url: base.to_owned(),
        })?;
    Ok((address, authority.to_owned()))
}

fn connect(address: SocketAddr) -> Result<TcpStream, HttpError> {
    let result = TcpStream::connect_timeout(&address, Duration::from_secs(30)).map_err(|source| {
        HttpError::Connect {
            address: address.to_string(),
            source,
        }
    });
    result
}

pub(crate) fn write_request(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    authority: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> Result<(), HttpError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(30))))?;
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {authority}\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    stream.flush().map_err(HttpError::Io)
}

pub(crate) fn read_response(stream: &mut TcpStream) -> Result<Response, HttpError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let chunk = buffer
            .get(..count)
            .ok_or_else(|| HttpError::Response("invalid read size".to_owned()))?;
        let size = bytes.len().checked_add(count).ok_or(HttpError::TooLarge)?;
        if size > MAX_BODY {
            return Err(HttpError::TooLarge);
        }
        bytes.extend_from_slice(chunk);
    }
    parse_response(&bytes)
}

pub(crate) fn parse_response(bytes: &[u8]) -> Result<Response, HttpError> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Response("missing HTTP headers".to_owned()))?;
    let header_bytes = bytes
        .get(..header_end)
        .ok_or_else(|| HttpError::Response("invalid header range".to_owned()))?;
    let header_text = str::from_utf8(header_bytes).map_err(HttpError::Text)?;
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| HttpError::Response("missing HTTP status".to_owned()))?
        .parse::<u16>()
        .map_err(|error| HttpError::Response(error.to_string()))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let body_start = header_end
        .checked_add(4)
        .ok_or_else(|| HttpError::Response("header length overflow".to_owned()))?;
    let body_bytes = bytes
        .get(body_start..)
        .ok_or_else(|| HttpError::Response("invalid body range".to_owned()))?;
    let body_bytes = if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunks(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    let body = parse_body(&headers, &body_bytes)?;
    Ok(Response {
        status,
        headers,
        body,
    })
}

pub(crate) fn decode_chunks(bytes: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut cursor = 0;
    let mut output = Vec::new();
    loop {
        let line_end = bytes
            .get(cursor..)
            .and_then(|rest| rest.windows(2).position(|window| window == b"\r\n"))
            .ok_or_else(|| HttpError::Response("invalid chunk size".to_owned()))?;
        let size_end = cursor.checked_add(line_end).ok_or(HttpError::TooLarge)?;
        let size_text = str::from_utf8(
            bytes
                .get(cursor..size_end)
                .ok_or_else(|| HttpError::Response("invalid chunk range".to_owned()))?,
        )
        .map_err(HttpError::Text)?;
        let size = usize::from_str_radix(size_text.trim(), 16)
            .map_err(|error| HttpError::Response(error.to_string()))?;
        if size == 0 {
            return Ok(output);
        }
        let data_start = size_end.checked_add(2).ok_or(HttpError::TooLarge)?;
        let data_end = data_start.checked_add(size).ok_or(HttpError::TooLarge)?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| HttpError::Response("short HTTP chunk".to_owned()))?;
        let new_size = output.len().checked_add(size).ok_or(HttpError::TooLarge)?;
        if new_size > MAX_BODY {
            return Err(HttpError::TooLarge);
        }
        output.extend_from_slice(data);
        cursor = data_end.checked_add(2).ok_or(HttpError::TooLarge)?;
    }
}

pub(crate) fn parse_body(
    headers: &BTreeMap<String, String>,
    bytes: &[u8],
) -> Result<Value, HttpError> {
    let text = str::from_utf8(bytes).map_err(HttpError::Text)?;
    if headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        let events = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|line| *line != "[DONE]")
            .map(serde_json::from_str)
            .collect::<Result<Vec<Value>, serde_json::Error>>()
            .map_err(HttpError::Body)?;
        return Ok(Value::Array(events));
    }
    serde_json::from_str(text).map_or_else(|_| Ok(Value::String(text.to_owned())), Ok)
}

impl From<Error> for HttpError {
    fn from(error: Error) -> Self {
        HttpError::Io(error)
    }
}
