use std::str;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];
const UTF32_LE_BOM: [u8; 4] = [0xFF, 0xFE, 0x00, 0x00];
const UTF32_BE_BOM: [u8; 4] = [0x00, 0x00, 0xFE, 0xFF];

#[derive(Debug, thiserror::Error)]
pub(crate) enum DecodeError {
    #[error("the body is not valid UTF-8")]
    Utf8,
    #[error("the body is truncated UTF-16")]
    TruncatedUtf16,
    #[error("the body is truncated UTF-32")]
    TruncatedUtf32,
    #[error("the body holds an invalid UTF-16 code unit")]
    Utf16Unit,
    #[error("the body holds an invalid UTF-32 code point")]
    CodePoint,
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

// json.loads reads the encoding exactly like json.detect_encoding in CPython
pub(crate) fn decode_json_bytes(body: &[u8]) -> Result<String, DecodeError> {
    if let Some(rest) = body.strip_prefix(&UTF32_LE_BOM) {
        return utf32(rest, Endian::Little);
    }
    if let Some(rest) = body.strip_prefix(&UTF32_BE_BOM) {
        return utf32(rest, Endian::Big);
    }
    if let Some(rest) = body.strip_prefix(&UTF16_LE_BOM) {
        return utf16(rest, Endian::Little);
    }
    if let Some(rest) = body.strip_prefix(&UTF16_BE_BOM) {
        return utf16(rest, Endian::Big);
    }
    if let Some(rest) = body.strip_prefix(&UTF8_BOM) {
        return utf8(rest);
    }
    match body {
        [0, 0, _, _, ..] => utf32(body, Endian::Big),
        [0, _, _, _, ..] => utf16(body, Endian::Big),
        [_, 0, 0, 0, ..] => utf32(body, Endian::Little),
        [_, 0, _, _, ..] => utf16(body, Endian::Little),
        _ => utf8(body),
    }
}

fn utf8(body: &[u8]) -> Result<String, DecodeError> {
    let Ok(text) = str::from_utf8(body) else {
        return Err(DecodeError::Utf8);
    };
    Ok(text.to_owned())
}

const fn partial_utf16(pair: [u8; 2], endian: Endian) -> u16 {
    match endian {
        Endian::Little => u16::from_le_bytes(pair),
        Endian::Big => u16::from_be_bytes(pair),
    }
}

const fn partial_utf32(quad: [u8; 4], endian: Endian) -> u32 {
    match endian {
        Endian::Little => u32::from_le_bytes(quad),
        Endian::Big => u32::from_be_bytes(quad),
    }
}

fn utf16(body: &[u8], endian: Endian) -> Result<String, DecodeError> {
    let (pairs, remainder) = body.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(DecodeError::TruncatedUtf16);
    }
    let units = pairs.iter().map(|pair| partial_utf16(*pair, endian));
    let mut text = String::new();
    for unit in char::decode_utf16(units) {
        match unit {
            Ok(character) => text.push(character),
            Err(_) => return Err(DecodeError::Utf16Unit),
        }
    }
    Ok(text)
}

fn utf32(body: &[u8], endian: Endian) -> Result<String, DecodeError> {
    if !body.len().is_multiple_of(4) {
        return Err(DecodeError::TruncatedUtf32);
    }
    body.as_chunks::<4>()
        .0
        .iter()
        .map(|quad| partial_utf32(*quad, endian))
        .map(char::from_u32)
        .collect::<Option<String>>()
        .ok_or(DecodeError::CodePoint)
}
