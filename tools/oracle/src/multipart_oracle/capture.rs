use std::str;

use crate::OracleError;

use super::{MAX_CAPTURED_BODY_BYTES, MAX_PARTS, Part};

pub fn decode_multipart_capture(content_type: &str, body: &[u8]) -> Result<Vec<Part>, OracleError> {
    if body.len() > MAX_CAPTURED_BODY_BYTES {
        return Err(OracleError::Multipart(
            "captured body exceeds 16777216 bytes",
        ));
    }
    let boundary = boundary(content_type).ok_or(OracleError::Multipart(
        "multipart capture has no valid boundary",
    ))?;
    let marker = format!("--{boundary}").into_bytes();
    decode_parts(body, &marker, &boundary)
}

fn decode_parts(body: &[u8], marker: &[u8], boundary: &str) -> Result<Vec<Part>, OracleError> {
    let mut parts = Vec::new();
    let mut cursor = 0;
    while let Some(start) = find_marker(body, cursor, marker) {
        let marker_end = start
            .checked_add(marker.len())
            .ok_or(OracleError::Multipart(
                "multipart capture has no valid boundary",
            ))?;
        if body.get(
            marker_end
                ..marker_end.checked_add(2).ok_or(OracleError::Multipart(
                    "multipart capture has no valid boundary",
                ))?,
        ) == Some(b"--")
        {
            break;
        }
        let content_start = marker_end.checked_add(2).ok_or(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ))?;
        let next = find_marker(body, content_start, marker).ok_or(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ))?;
        parts.push(parse_part(body.get(content_start..next).ok_or(
            OracleError::Multipart("multipart capture has no valid boundary"),
        )?)?);
        if parts.len() > MAX_PARTS {
            return Err(OracleError::Multipart(
                "multipart contains more than 128 parts",
            ));
        }
        cursor = next;
    }
    if parts.is_empty() && !body.ends_with(format!("--{boundary}--\r\n").as_bytes()) {
        return Err(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ));
    }
    Ok(parts)
}

fn find_marker(body: &[u8], start: usize, marker: &[u8]) -> Option<usize> {
    let offset = body
        .get(start..)?
        .windows(marker.len())
        .position(|window| window == marker)?;
    start.checked_add(offset)
}

fn parse_part(raw_section: &[u8]) -> Result<Part, OracleError> {
    let section = raw_section.strip_suffix(b"\r\n").unwrap_or(raw_section);
    let split = section
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ))?;
    let headers = section.get(..split).ok_or(OracleError::Multipart(
        "multipart capture has no valid boundary",
    ))?;
    let payload_start = split.checked_add(4).ok_or(OracleError::Multipart(
        "multipart capture has no valid boundary",
    ))?;
    let payload = section.get(payload_start..).ok_or(OracleError::Multipart(
        "multipart capture has no valid boundary",
    ))?;
    let (name, filename, content_type) = parse_headers(headers)?;
    Ok(Part {
        name,
        filename,
        content_type,
        body: payload.to_vec(),
    })
}

fn parse_headers(headers: &[u8]) -> Result<(String, Option<String>, Option<String>), OracleError> {
    let mut name = None;
    let mut filename = None;
    let mut part_content_type = None;
    for header_line in headers.split(|byte| *byte == b'\n') {
        let line = header_line.strip_suffix(b"\r").unwrap_or(header_line);
        match line {
            line_value if line_value.starts_with(b"Content-Disposition: form-data; ") => {
                let disposition = line_value
                    .strip_prefix(b"Content-Disposition: form-data; ")
                    .ok_or(OracleError::Multipart(
                        "multipart capture has no valid boundary",
                    ))?;
                let (part_name, part_filename) = parse_disposition(disposition)?;
                name = part_name.or(name);
                filename = part_filename.or(filename);
            }
            line_value if line_value.starts_with(b"Content-Type: ") => {
                let value =
                    line_value
                        .strip_prefix(b"Content-Type: ")
                        .ok_or(OracleError::Multipart(
                            "multipart capture has no valid boundary",
                        ))?;
                part_content_type = Some(String::from_utf8(value.to_vec()).map_err(|_error| {
                    OracleError::Multipart("multipart capture has no valid boundary")
                })?);
            }
            _ => {}
        }
    }
    Ok((
        name.ok_or(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ))?,
        filename,
        part_content_type,
    ))
}

fn parse_disposition(disposition: &[u8]) -> Result<(Option<String>, Option<String>), OracleError> {
    let attributes = disposition
        .split(|byte| *byte == b';')
        .map(|attribute| attribute.strip_prefix(b" ").unwrap_or(attribute))
        .collect::<Vec<_>>();
    let name = find_attribute(&attributes, b"name=\"")?;
    let filename = find_attribute(&attributes, b"filename=\"")?;
    Ok((name, filename))
}

fn find_attribute(attributes: &[&[u8]], prefix: &[u8]) -> Result<Option<String>, OracleError> {
    for attribute in attributes {
        if let Some(value) = quoted_attribute(attribute, prefix) {
            return Ok(Some(value?));
        }
    }
    Ok(None)
}

fn quoted_attribute(attribute: &[u8], prefix: &[u8]) -> Option<Result<String, OracleError>> {
    let value = attribute.strip_prefix(prefix)?.strip_suffix(b"\"")?;
    Some(
        String::from_utf8(value.to_vec())
            .map_err(|_error| OracleError::Multipart("multipart capture has no valid boundary")),
    )
}

pub fn decode_capture_input(input: &[u8]) -> Result<Vec<Part>, OracleError> {
    let prefix_bytes = input.get(..2).ok_or(OracleError::Multipart(
        "multipart capture has no valid boundary",
    ))?;
    let prefix = <[u8; 2]>::try_from(prefix_bytes)
        .map_err(|_error| OracleError::Multipart("multipart capture has no valid boundary"))?;
    let content_type_length = usize::from(u16::from_be_bytes(prefix));
    let content_type_bytes =
        input
            .get(2..2 + content_type_length)
            .ok_or(OracleError::Multipart(
                "multipart capture has no valid boundary",
            ))?;
    let content_type = str::from_utf8(content_type_bytes)
        .map_err(|_error| OracleError::Multipart("multipart capture has no valid boundary"))?;
    let body = input
        .get(2 + content_type_length..)
        .ok_or(OracleError::Multipart(
            "multipart capture has no valid boundary",
        ))?;
    decode_multipart_capture(content_type, body)
}

fn boundary(content_type: &str) -> Option<String> {
    let (media_type, parameters) = content_type.split_once(';')?;
    if !media_type
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    let boundary_value = parameters
        .split(';')
        .find_map(|item| item.trim().strip_prefix("boundary="))?
        .trim();
    let unquoted_boundary = if let Some(quoted) = boundary_value.strip_prefix('"') {
        quoted.strip_suffix('"')?
    } else {
        boundary_value
    };
    (!unquoted_boundary.is_empty()
        && !unquoted_boundary.bytes().any(|byte| {
            byte.is_ascii_whitespace()
                || byte == b'\r'
                || byte == b'\n'
                || byte == b'"'
                || byte == b';'
        }))
    .then(|| unquoted_boundary.to_owned())
}
