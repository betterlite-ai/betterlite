use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Read as _, Write as _};
use std::net::TcpStream;
use std::str;
use std::time::Duration;

use serde_json::Value;

use super::ProviderRequest;

pub(crate) const MAX_REQUEST: usize = 16 * 1024 * 1024;

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Option<Value>,
    pub(crate) events: Option<Vec<String>>,
}

pub(crate) fn handle_connection(
    stream: &mut TcpStream,
    answer: &Answer,
) -> Result<Option<ProviderRequest>, Error> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let request = read_request(stream)?;
    if is_model_probe(&request) {
        let response_body = br#"{"object":"list","data":[]}"#;
        write_response(
            stream,
            200,
            &BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            response_body,
        )?;
        return Ok(None);
    }
    let response_body = if let Some(body) = &answer.body {
        serde_json::to_vec(body).map_err(Error::other)?
    } else {
        let mut body = Vec::new();
        for event in answer.events.as_deref().unwrap_or_default() {
            body.extend_from_slice(b"data: ");
            body.extend_from_slice(event.as_bytes());
            body.extend_from_slice(b"\n\n");
        }
        body
    };
    write_response(stream, answer.status, &answer.headers, &response_body)?;
    Ok(Some(request))
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &BTreeMap<String, String>,
    response_body: &[u8],
) -> Result<(), Error> {
    let reason = if (200..300).contains(&status) {
        "OK"
    } else {
        "Error"
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(
        stream,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    )?;
    stream.write_all(response_body)?;
    stream.flush()?;
    Ok(())
}

pub(crate) fn read_request(stream: &mut TcpStream) -> Result<ProviderRequest, Error> {
    let bytes = read_bytes(stream)?;
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "missing HTTP headers"))?;
    let header_bytes = bytes
        .get(..header_end)
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "invalid HTTP header range"))?;
    let header_text = str::from_utf8(header_bytes).map_err(Error::other)?;
    let mut lines = header_text.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "missing HTTP request target"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "missing HTTP method"))?;
    let path = request_parts
        .next()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "missing HTTP path"))?;
    let (headers, content_length) = request_headers(lines)?;
    let body_start = header_end
        .checked_add(4)
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "HTTP header length overflow"))?;
    let body_end = body_start
        .checked_add(content_length)
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "HTTP body length overflow"))?;
    let body_bytes = bytes
        .get(body_start..body_end)
        .ok_or_else(|| Error::new(ErrorKind::UnexpectedEof, "short HTTP request body"))?;
    let body = if body_bytes.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(body_bytes) {
            Ok(value) => value,
            Err(_) => Value::String(str::from_utf8(body_bytes).map_err(Error::other)?.to_owned()),
        }
    };
    Ok(ProviderRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body,
    })
}

pub(crate) fn read_bytes(stream: &mut TcpStream) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    while should_read_more(bytes.len()) {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let chunk = buffer
            .get(..count)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "invalid read size"))?;
        bytes.extend_from_slice(chunk);
        if header_complete(&bytes) {
            break;
        }
    }
    Ok(bytes)
}

pub(crate) fn should_read_more(length: usize) -> bool {
    let should_read = length < MAX_REQUEST;
    should_read
}

pub(crate) fn is_model_probe(request: &ProviderRequest) -> bool {
    let probe = request.method == "GET" && request.path == "/v1/models";
    probe
}

pub(crate) fn header_complete(bytes: &[u8]) -> bool {
    let complete = bytes.windows(4).any(|window| window == b"\r\n\r\n");
    complete
}

pub(crate) fn request_headers<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<(BTreeMap<String, String>, usize), Error> {
    let mut headers = BTreeMap::new();
    let mut content_length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let header_name = name.trim().to_ascii_lowercase();
            let header_value = value.trim().to_owned();
            if header_name == "content-length" {
                content_length = header_value.parse::<usize>().map_err(Error::other)?;
            }
            headers.insert(header_name, header_value);
        }
    }
    Ok((headers, content_length))
}

pub(crate) fn to_answer(answer: &crate::case::UpstreamAnswer) -> Answer {
    let prepared = Answer {
        status: answer.status,
        headers: answer.headers.clone(),
        body: answer.body.clone(),
        events: answer.events.clone(),
    };
    prepared
}
