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
        let mut fields = part.splitn(2, |byte| *byte == b'=');
        let name = fields.next().unwrap_or_default();
        let value = fields.next().unwrap_or_default();
        items.push(FormPart {
            name: unquote_plus_latin1(name),
            value: PartValue::Text(unquote_plus_latin1(value)),
        });
    }
    Ok(FormParts { items })
}

// Starlette reads each field via unquote_plus(bytes.decode("latin-1")): raw bytes stay latin-1 characters and percent escapes decode as UTF-8 runs with one U+FFFD per invalid sequence
fn unquote_plus_latin1(bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    let mut escaped = Vec::with_capacity(bytes.len());
    let mut remainder = bytes;
    while let Some((byte, rest)) = remainder.split_first() {
        remainder = rest;
        if *byte >= 0x80_u8 {
            push_replaced_utf8(&mut decoded, &escaped);
            escaped.clear();
            decoded.push(char::from(*byte));
        } else if *byte == b'+' {
            escaped.push(b' ');
        } else if *byte == b'%' {
            match rest {
                [high, low, tail @ ..] if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() => {
                    escaped.push(
                        hex_value(*high)
                            .wrapping_shl(4_u32)
                            .wrapping_add(hex_value(*low)),
                    );
                    remainder = tail;
                }
                _ => escaped.push(b'%'),
            }
        } else {
            escaped.push(*byte);
        }
    }
    push_replaced_utf8(&mut decoded, &escaped);
    decoded
}

// Python's errors="replace" decodes each maximal invalid subsequence as one U+FFFD
fn push_replaced_utf8(decoded: &mut String, bytes: &[u8]) {
    for chunk in bytes.utf8_chunks() {
        decoded.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            decoded.push(char::REPLACEMENT_CHARACTER);
        }
    }
}

const fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte.wrapping_sub(b'0'),
        b'a'..=b'f' => byte.wrapping_sub(b'a').wrapping_add(10_u8),
        _ => byte.wrapping_sub(b'A').wrapping_add(10_u8),
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
