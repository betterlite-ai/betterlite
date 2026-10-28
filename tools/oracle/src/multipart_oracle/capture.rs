use std::str;

use super::error::MultipartError;
use super::{LIMITS, Part};

pub fn decode_multipart_capture(
    content_type: &str,
    body: &[u8],
) -> Result<Vec<Part>, MultipartError> {
    if body.len() > LIMITS.captured_body_bytes {
        return Err(MultipartError::CapturedTooLarge {
            limit: LIMITS.captured_body_bytes,
        });
    }
    let boundary = boundary(content_type).ok_or(MultipartError::Boundary)?;
    let delimiter = format!("\r\n--{boundary}");
    let opening = delimiter.get(2..).ok_or(MultipartError::Boundary)?;
    let mut rest = body
        .strip_prefix(opening.as_bytes())
        .ok_or(MultipartError::Boundary)?;
    let mut parts = Vec::new();
    loop {
        if let Some(epilogue) = rest.strip_prefix(b"--") {
            return match epilogue {
                b"" | b"\r\n" => Ok(parts),
                _ => Err(MultipartError::Boundary),
            };
        }
        let section_start = rest.strip_prefix(b"\r\n").ok_or(MultipartError::Boundary)?;
        let end = section_start
            .windows(delimiter.len())
            .position(|window| window == delimiter.as_bytes())
            .ok_or(MultipartError::Boundary)?;
        let (section, next) = section_start
            .split_at_checked(end)
            .ok_or(MultipartError::Boundary)?;
        if parts.len() == LIMITS.parts {
            return Err(MultipartError::TooManyParts {
                limit: LIMITS.parts,
            });
        }
        parts.push(parse_part(section)?);
        rest = next
            .strip_prefix(delimiter.as_bytes())
            .ok_or(MultipartError::Boundary)?;
    }
}

pub fn split_capture_input(input: &[u8]) -> Result<(&str, &[u8]), MultipartError> {
    let (prefix, rest) = input
        .split_first_chunk::<2>()
        .ok_or(MultipartError::CaptureInput)?;
    let (content_type, body) = rest
        .split_at_checked(usize::from(u16::from_be_bytes(*prefix)))
        .ok_or(MultipartError::CaptureInput)?;
    let text = str::from_utf8(content_type).map_err(|_source| MultipartError::Boundary)?;
    Ok((text, body))
}

fn parse_part(section: &[u8]) -> Result<Part, MultipartError> {
    let split = section
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(MultipartError::Boundary)?;
    let (headers, payload) = section
        .split_at_checked(split)
        .ok_or(MultipartError::Boundary)?;
    let mut part = Part {
        name: String::new(),
        filename: None,
        content_type: None,
        body: payload.get(4..).ok_or(MultipartError::Boundary)?.to_vec(),
    };
    let mut disposition_seen = false;
    for raw_line in headers.split(|byte| *byte == b'\n') {
        let line = str::from_utf8(raw_line.strip_suffix(b"\r").unwrap_or(raw_line))
            .map_err(|_source| MultipartError::Boundary)?;
        let unsupported = || MultipartError::PartHeader {
            line: line.to_owned(),
        };
        let (name, value) = line.split_once(": ").ok_or_else(unsupported)?;
        if name.eq_ignore_ascii_case("content-disposition") && !disposition_seen {
            disposition_seen = true;
            let attributes = value.strip_prefix("form-data; ").ok_or_else(unsupported)?;
            parse_disposition(attributes, &mut part).ok_or_else(unsupported)?;
        } else if name.eq_ignore_ascii_case("content-type") && part.content_type.is_none() {
            part.content_type = Some(value.to_owned());
        } else {
            return Err(unsupported());
        }
    }
    if part.name.is_empty() {
        return Err(MultipartError::PartName);
    }
    Ok(part)
}

fn parse_disposition(attributes: &str, part: &mut Part) -> Option<()> {
    for attribute in attributes.split("; ") {
        let (key, quoted) = attribute.split_once('=')?;
        let value = quoted.strip_prefix('"')?.strip_suffix('"')?.to_owned();
        match key {
            "name" if part.name.is_empty() => part.name = value,
            "filename" if part.filename.is_none() => part.filename = Some(value),
            _ => return None,
        }
    }
    Some(())
}

fn boundary(content_type: &str) -> Option<&str> {
    let (media_type, parameters) = content_type.split_once(';')?;
    if !media_type
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    let value = parameters
        .split(';')
        .find_map(|item| item.trim().strip_prefix("boundary="))?;
    let unquoted = match value.strip_prefix('"') {
        Some(quoted) => quoted.strip_suffix('"')?,
        None => value,
    };
    let valid = (1..=70).contains(&unquoted.len())
        && unquoted
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&byte))
        && !unquoted.ends_with(' ');
    valid.then_some(unquoted)
}
