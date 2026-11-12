use betterlite_models::decisions::Json;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];
const UTF32_LE_BOM: [u8; 4] = [0xFF, 0xFE, 0x00, 0x00];
const UTF32_BE_BOM: [u8; 4] = [0x00, 0x00, 0xFE, 0xFF];
// the ASCII boundaries bytes.splitlines() splits on
const LINE_BREAKS: [u8; 7] = [b'\r', b'\n', 0x0b, 0x0c, 0x1c, 0x1d, 0x1e];
// bytes.strip() drops the whitespace bytes.isspace() accepts
const STRIPPED: [u8; 6] = [b' ', b'\t', b'\n', b'\r', 0x0b, 0x0c];

#[derive(Debug, Clone, Copy)]
enum Order {
    Little,
    Big,
}

pub(super) fn is_message_stop_bytes(bytes: &[u8]) -> bool {
    for line in split_lines(bytes) {
        if line == b"event: message_stop" {
            return true;
        }
    }
    false
}

pub(super) fn is_content_delta_bytes(bytes: &[u8]) -> bool {
    for line in split_lines(bytes) {
        if line == b"event: content_block_delta" {
            return true;
        }
    }
    false
}

pub(super) fn is_ping_bytes(bytes: &[u8]) -> bool {
    let event_lines: Vec<&[u8]> = split_lines(bytes)
        .into_iter()
        .filter(|line| line.starts_with(b"event:"))
        .collect();
    if event_lines.is_empty() {
        return false;
    }
    event_lines.iter().all(|line| *line == b"event: ping")
        && bytes.starts_with(b"event: ping")
        && (bytes.ends_with(b"\n\n") || bytes.ends_with(b"\r\n\r\n"))
}

pub(super) fn data_line_payloads(bytes: &[u8]) -> Vec<Json> {
    let mut payloads = Vec::new();
    for line in split_lines(bytes) {
        let Some(data) = line.strip_prefix(b"data:") else {
            continue;
        };
        let Some(text) = decode_text_bytes(strip_whitespace(data)) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Json>(&text) {
            payloads.push(value);
        }
    }
    payloads
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut index = 0_usize;
    while let Some(byte) = bytes.get(index) {
        if !LINE_BREAKS.contains(byte) {
            index = index.saturating_add(1);
            continue;
        }
        let width = if *byte == b'\r' && bytes.get(index.saturating_add(1)) == Some(&b'\n') {
            2
        } else {
            1
        };
        if let Some(line) = bytes.get(start..index) {
            lines.push(line);
        }
        start = index.saturating_add(width);
        index = start;
    }
    let tail = bytes.get(start..).unwrap_or_default();
    if !tail.is_empty() {
        lines.push(tail);
    }
    lines
}

fn strip_whitespace(bytes: &[u8]) -> &[u8] {
    let Some(first) = bytes.iter().position(|byte| !STRIPPED.contains(byte)) else {
        return &[];
    };
    let Some(last) = bytes.iter().rposition(|byte| !STRIPPED.contains(byte)) else {
        return &[];
    };
    bytes.get(first..=last).unwrap_or_default()
}

// json.loads decodes bytes with json.detect_encoding, so a BOM selects UTF-8, UTF-16 or UTF-32
fn decode_text_bytes(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&UTF32_LE_BOM) {
        return decode_utf32(rest, Order::Little);
    }
    if let Some(rest) = bytes.strip_prefix(&UTF32_BE_BOM) {
        return decode_utf32(rest, Order::Big);
    }
    if let Some(rest) = bytes.strip_prefix(&UTF16_LE_BOM) {
        return decode_utf16(rest, Order::Little);
    }
    if let Some(rest) = bytes.strip_prefix(&UTF16_BE_BOM) {
        return decode_utf16(rest, Order::Big);
    }
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        return str::from_utf8(rest).ok().map(str::to_owned);
    }
    match bytes {
        [0, 0, _, _, ..] => decode_utf32(bytes, Order::Big),
        [0, _, ..] => decode_utf16(bytes, Order::Big),
        [_, 0, 0, 0, ..] => decode_utf32(bytes, Order::Little),
        [_, 0, ..] => decode_utf16(bytes, Order::Little),
        _ => str::from_utf8(bytes).ok().map(str::to_owned),
    }
}

fn decode_utf16(bytes: &[u8], order: Order) -> Option<String> {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs
        .iter()
        .map(|pair| match order {
            Order::Little => u16::from_le_bytes(*pair),
            Order::Big => u16::from_be_bytes(*pair),
        })
        .collect();
    String::from_utf16(&units).ok()
}

fn decode_utf32(bytes: &[u8], order: Order) -> Option<String> {
    let (quads, remainder) = bytes.as_chunks::<4>();
    if !remainder.is_empty() {
        return None;
    }
    let mut text = String::new();
    for quad in quads {
        let value = match order {
            Order::Little => u32::from_le_bytes(*quad),
            Order::Big => u32::from_be_bytes(*quad),
        };
        text.push(char::from_u32(value)?);
    }
    Some(text)
}
