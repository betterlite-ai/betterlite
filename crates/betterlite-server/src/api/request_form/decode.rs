use betterlite_core::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use betterlite_core::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};
use std::str;

use super::RequestFormError;
use crate::api::ocr_request::multipart_form::charset::decode_part;
use crate::api::ocr_request::multipart_form::disposition::disposition_param_bytes;
use crate::api::ocr_request::multipart_form::raw_split::{RawItem, split_items};

pub(super) fn urlencoded(bytes: &[u8], limits: &ApiLimits) -> Result<FormParts, RequestFormError> {
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

pub(super) fn multipart(
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
