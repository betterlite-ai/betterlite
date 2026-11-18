use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimits, ItemsBoundary, JsonBoundary,
};
use betterlite_core::api::request_body::{
    DecodedJson, MediaTypeKind, RequestBodyError, classify_media_type,
};
use betterlite_core::api::request_form::values::{
    FormFields, FormPart, FormParts, PartValue, UploadFile, get_form_data,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestFormError {
    #[error("request form body is too large")]
    BodyTooLarge,
    #[error("request form has too many parts")]
    TooManyParts,
    #[error("invalid request form content type")]
    ContentType,
    #[error("invalid request form body")]
    InvalidBody,
    #[error("request form field name is missing")]
    MissingName,
    #[error("request form field is not valid UTF-8")]
    InvalidText,
    #[error("unsupported content type: {0}")]
    UnsupportedContentType(String),
    #[error(transparent)]
    RequestBody(#[from] RequestBodyError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestFormBody {
    Empty,
    Json(DecodedJson),
    Form(FormFields),
}

pub fn get_request_body(
    method: &str,
    content_type: &str,
    body: &[u8],
    limits: &ApiLimits,
) -> Result<RequestFormBody, RequestFormError> {
    if method != "POST" {
        return Ok(RequestFormBody::Empty);
    }
    match classify_media_type(content_type) {
        MediaTypeKind::Json => Ok(RequestFormBody::Json(
            super::request_body::decode_request_body(body, limits)?,
        )),
        MediaTypeKind::Form => {
            let parts = decode_request_form(body, content_type.as_bytes(), limits)?;
            Ok(RequestFormBody::Form(
                get_form_data(&parts).map_err(|_error| RequestFormError::InvalidBody)?,
            ))
        }
        MediaTypeKind::Binary | MediaTypeKind::Other => Err(
            RequestFormError::UnsupportedContentType(content_type.to_owned()),
        ),
    }
}

/// Decode URL encoded and multipart form bytes while retaining repeated field order.
pub fn decode_request_form(
    bytes: &[u8],
    content_type: &[u8],
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: content_type.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    let content_type =
        std::str::from_utf8(content_type).map_err(|_error| RequestFormError::ContentType)?;
    let (media_type, params) = content_type.split_once(';').unwrap_or((content_type, ""));
    match media_type.trim().to_ascii_lowercase().as_str() {
        "application/x-www-form-urlencoded" => decode_urlencoded(bytes, limits),
        "multipart/form-data" => {
            let boundary = boundary_parameter(params).ok_or(RequestFormError::ContentType)?;
            decode_multipart(bytes, &boundary, limits)
        }
        _other => Err(RequestFormError::ContentType),
    }
}

fn decode_urlencoded(bytes: &[u8], limits: &ApiLimits) -> Result<FormParts, RequestFormError> {
    let part_count = if bytes.is_empty() {
        0
    } else {
        bytes
            .iter()
            .filter(|byte| **byte == b'&')
            .count()
            .saturating_add(1)
    };
    check_part_count(part_count, limits)?;
    let mut items = Vec::with_capacity(part_count);
    for (name, value) in form_urlencoded::parse(bytes) {
        check_part_count(items.len() + 1, limits)?;
        items.push(FormPart {
            name: name.into_owned(),
            value: PartValue::Text(value.into_owned()),
        });
    }
    Ok(FormParts { items })
}

fn decode_multipart(
    bytes: &[u8],
    boundary: &str,
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    let marker = format!("--{boundary}").into_bytes();
    let mut remaining = bytes;
    let mut items = Vec::new();
    let mut ended = false;
    loop {
        let Some((start, marker_end)) = find_boundary(remaining, &marker) else {
            break;
        };
        remaining = &remaining[start + marker_end..];
        if marker_end == marker.len() + 2 {
            ended = true;
            break;
        }
        remaining = remaining
            .strip_prefix(b"\r\n")
            .ok_or(RequestFormError::InvalidBody)?;
        let header_end =
            find_subslice(remaining, b"\r\n\r\n").ok_or(RequestFormError::InvalidBody)?;
        let headers = parse_headers(&remaining[..header_end])?;
        let payload_start = header_end + 4;
        let (next_boundary, _end) = find_boundary(&remaining[payload_start..], &marker)
            .ok_or(RequestFormError::InvalidBody)?;
        let payload_end = payload_start + next_boundary - 2;
        let (name, filename) = disposition(&headers)?;
        let payload = &remaining[payload_start..payload_end];
        check_part_count(items.len() + 1, limits)?;
        let value = if let Some(filename) = filename {
            let content_type = headers
                .iter()
                .find(|(key, _value)| key.eq_ignore_ascii_case("content-type"))
                .map(|(_key, value)| value.clone());
            PartValue::Upload(UploadFile {
                filename,
                content_type,
                content: payload.to_vec(),
                size: Some(
                    u64::try_from(payload.len())
                        .map_err(|_error| RequestFormError::BodyTooLarge)?,
                ),
            })
        } else {
            PartValue::Text(
                std::str::from_utf8(payload)
                    .map_err(|_error| RequestFormError::InvalidText)?
                    .to_owned(),
            )
        };
        items.push(FormPart { name, value });
        remaining = &remaining[payload_start + next_boundary..];
    }
    if !ended && !bytes.is_empty() {
        return Err(RequestFormError::InvalidBody);
    }
    Ok(FormParts { items })
}

fn check_part_count(count: usize, limits: &ApiLimits) -> Result<(), RequestFormError> {
    limits
        .check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: count,
        }))
        .map(|_result| ())
        .map_err(|_error| RequestFormError::TooManyParts)
}

fn boundary_parameter(params: &str) -> Option<String> {
    params.split(';').find_map(|parameter| {
        let (key, value) = parameter.trim().split_once('=')?;
        if !key.eq_ignore_ascii_case("boundary") {
            return None;
        }
        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        (!unquoted.is_empty()).then(|| unquoted.to_owned())
    })
}

fn parse_headers(bytes: &[u8]) -> Result<Vec<(String, String)>, RequestFormError> {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let line = std::str::from_utf8(line).map_err(|_error| RequestFormError::InvalidBody)?;
            let (key, value) = line.split_once(':').ok_or(RequestFormError::InvalidBody)?;
            Ok((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn disposition(headers: &[(String, String)]) -> Result<(String, Option<String>), RequestFormError> {
    let header = headers
        .iter()
        .find(|(key, _value)| key.eq_ignore_ascii_case("content-disposition"))
        .map(|(_key, value)| value)
        .ok_or(RequestFormError::MissingName)?;
    let mut name = None;
    let mut filename = None;
    for parameter in header.split(';').skip(1) {
        let Some((key, value)) = parameter.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "name" => name = Some(value.to_owned()),
            "filename" => filename = Some(value.to_owned()),
            _other => {}
        }
    }
    Ok((
        name.filter(|value| !value.is_empty())
            .ok_or(RequestFormError::MissingName)?,
        filename,
    ))
}

fn find_boundary(haystack: &[u8], marker: &[u8]) -> Option<(usize, usize)> {
    for (index, window) in haystack.windows(marker.len()).enumerate() {
        if window != marker || (index != 0 && (index < 2 || &haystack[index - 2..index] != b"\r\n"))
        {
            continue;
        }
        let trailer = &haystack[index + marker.len()..];
        if trailer.starts_with(b"--") {
            return Some((index, marker.len() + 2));
        }
        if trailer.starts_with(b"\r\n") {
            return Some((index, marker.len()));
        }
    }
    None
}

#[cfg(test)]
mod tests;
