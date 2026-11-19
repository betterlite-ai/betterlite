use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimits, ItemsBoundary, JsonBoundary,
};
use betterlite_core::api::request_body::{
    DecodedJson, MediaTypeKind, RequestBodyError, classify_media_type,
};
use betterlite_core::api::request_form::values::{
    FormFields, FormPart, FormParts, PartValue, UploadFile, get_form_data,
};
use std::str;
use thiserror::Error;

use crate::api::ocr_request::multipart_form::charset::decode_part;
use crate::api::ocr_request::multipart_form::disposition::disposition_param_bytes;
use crate::api::ocr_request::multipart_form::raw_split::{RawItem, split_items};

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
    #[error("Unsupported content type: {0}")]
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

// decode_request_form: URL encoded and multipart bytes decode into ordered parts before any field policy runs
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
    let header_text =
        str::from_utf8(content_type).map_err(|_error| RequestFormError::ContentType)?;
    let media_type = header_text.split(';').next().unwrap_or(header_text);
    match media_type.trim().to_ascii_lowercase().as_str() {
        "application/x-www-form-urlencoded" => decode_urlencoded(bytes, limits),
        "multipart/form-data" => decode_multipart(bytes, header_text, limits),
        _other => Err(RequestFormError::ContentType),
    }
}

fn decode_urlencoded(bytes: &[u8], limits: &ApiLimits) -> Result<FormParts, RequestFormError> {
    let part_count = if bytes.is_empty() {
        0
    } else {
        bytes.split(|byte| *byte == b'&').count()
    };
    check_part_count(part_count, limits)?;
    let mut items = Vec::with_capacity(part_count);
    for part in bytes
        .split(|byte| *byte == b'&')
        .filter(|part| !part.is_empty())
    {
        let (name, value) = match part.iter().position(|byte| *byte == b'=') {
            Some(index) => (&part[..index], &part[index + 1..]),
            None => (part, &[][..]),
        };
        items.push(FormPart {
            name: unquote_plus_latin1(name),
            value: PartValue::Text(unquote_plus_latin1(value)),
        });
    }
    Ok(FormParts { items })
}

// Starlette reads each field via unquote_plus(bytes.decode("latin-1")), so raw bytes stay latin-1
fn unquote_plus_latin1(bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] >= 0x80 {
            decoded.push(char::from(bytes[index]));
            index += 1;
            continue;
        }
        let mut escaped = Vec::with_capacity(4);
        while index < bytes.len() && bytes[index] < 0x80 {
            let byte = bytes[index];
            if byte == b'+' {
                escaped.push(b' ');
                index += 1;
            } else if byte == b'%'
                && index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit()
            {
                escaped.push((hex_value(bytes[index + 1]) << 4) | hex_value(bytes[index + 2]));
                index += 3;
            } else {
                escaped.push(byte);
                index += 1;
            }
        }
        decoded.push_str(&String::from_utf8_lossy(&escaped));
    }
    decoded
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

fn decode_multipart(
    bytes: &[u8],
    content_type: &str,
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    let boundary_bytes = disposition_param_bytes(content_type.as_bytes(), "boundary")
        .filter(|value| !value.is_empty())
        .ok_or(RequestFormError::ContentType)?;
    let boundary =
        str::from_utf8(&boundary_bytes).map_err(|_error| RequestFormError::ContentType)?;
    let charset = disposition_param_bytes(content_type.as_bytes(), "charset")
        .map_or_else(|| "utf-8".to_owned(), |value| decode_part(&value, "latin1"));
    if bytes.is_empty() {
        return Ok(FormParts { items: Vec::new() });
    }
    let mut items = Vec::new();
    for item in split_items(bytes, boundary).map_err(|_error| RequestFormError::InvalidBody)? {
        let raw_item = item.map_err(|_error| RequestFormError::InvalidBody)?;
        check_part_count(items.len().saturating_add(1), limits)?;
        items.push(read_part(&raw_item, &charset)?);
    }
    Ok(FormParts { items })
}

fn read_part(item: &RawItem<'_>, charset: &str) -> Result<FormPart, RequestFormError> {
    let disposition = item
        .content_disposition
        .ok_or(RequestFormError::InvalidBody)?;
    let name_bytes =
        disposition_param_bytes(disposition, "name").ok_or(RequestFormError::MissingName)?;
    let name = decode_part(&name_bytes, charset);
    let value = match disposition_param_bytes(disposition, "filename") {
        Some(filename) => PartValue::Upload(UploadFile {
            filename: decode_part(&filename, charset),
            content_type: item.content_type.map(|value| decode_part(value, "latin1")),
            content: item.data.to_vec(),
            size: Some(
                u64::try_from(item.data.len()).map_err(|_error| RequestFormError::BodyTooLarge)?,
            ),
        }),
        None => PartValue::Text(decode_part(item.data, charset)),
    };
    Ok(FormPart { name, value })
}

fn check_part_count(count: usize, limits: &ApiLimits) -> Result<(), RequestFormError> {
    limits
        .check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: count,
        }))
        .map_err(|_error| RequestFormError::TooManyParts)?;
    Ok(())
}

mod tests;
