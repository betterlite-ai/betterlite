use std::io::ErrorKind;
use std::io::Read;

use crate::error::FixtureError;

pub struct Request {
    pub line: String,
    pub head: String,
    pub body: String,
}

pub fn read_request<Stream: Read>(stream: &mut Stream) -> Result<Request, FixtureError> {
    let mut raw_head = Vec::new();
    while !raw_head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|error| match error.kind() {
            ErrorKind::UnexpectedEof if raw_head.is_empty() => FixtureError::NoRequest,
            _other => FixtureError::Io(error),
        })?;
        raw_head.extend_from_slice(&byte);
    }
    let head = String::from_utf8(raw_head)?;
    let mut raw_body = vec![0; content_length(&head)?];
    stream.read_exact(&mut raw_body)?;
    Ok(Request {
        line: head.lines().next().unwrap_or_default().to_owned(),
        head,
        body: String::from_utf8(raw_body)?,
    })
}

fn content_length(head: &str) -> Result<usize, FixtureError> {
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return Ok(value.trim().parse()?);
        }
    }
    Ok(0)
}

mod tests;
