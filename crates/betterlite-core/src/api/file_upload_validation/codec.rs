const NON_ASCII: u8 = 0x80;
const MAX_CODE_POINT: u32 = 0x10FFFF;
#[derive(Debug, Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}
// Python's json.loads(bytes) picks the encoding per line with json.detect_encoding
pub(super) fn decode_line(line: &[u8]) -> Option<Vec<u8>> {
    let (encoding, skip) = detect_encoding(line);
    let body = line.get(skip..)?;
    match encoding {
        Encoding::Utf8 => decode_utf8(body),
        Encoding::Utf16Le => decode_units(body, 2, true),
        Encoding::Utf16Be => decode_units(body, 2, false),
        Encoding::Utf32Le => decode_units(body, 4, true),
        Encoding::Utf32Be => decode_units(body, 4, false),
    }
}
fn detect_encoding(line: &[u8]) -> (Encoding, usize) {
    if line.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        return (Encoding::Utf32Be, 4);
    }
    if line.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        return (Encoding::Utf32Le, 4);
    }
    if line.starts_with(&[0xFE, 0xFF]) {
        return (Encoding::Utf16Be, 2);
    }
    if line.starts_with(&[0xFF, 0xFE]) {
        return (Encoding::Utf16Le, 2);
    }
    if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (Encoding::Utf8, 3);
    }
    if line.len() >= 4 {
        let first = line.first().copied().unwrap_or_default();
        let second = line.get(1).copied().unwrap_or_default();
        if first == 0 {
            let sniffed = match second {
                0 => Encoding::Utf32Be,
                _ => Encoding::Utf16Be,
            };
            return (sniffed, 0);
        }
        if second == 0 {
            let third = line.get(2).copied().unwrap_or_default();
            let fourth = line.get(3).copied().unwrap_or_default();
            let sniffed = if third == 0 && fourth == 0 {
                Encoding::Utf32Le
            } else {
                Encoding::Utf16Le
            };
            return (sniffed, 0);
        }
    }
    if line.len() == 2 {
        let first = line.first().copied().unwrap_or_default();
        let second = line.get(1).copied().unwrap_or_default();
        if first == 0 {
            return (Encoding::Utf16Be, 0);
        }
        if second == 0 {
            return (Encoding::Utf16Le, 0);
        }
    }
    (Encoding::Utf8, 0)
}
fn decode_utf8(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((first, tail)) = rest.split_first() {
        let (width, minimum, maximum) = match *first {
            0x00..=0x7F => {
                decoded.push(*first);
                rest = tail;
                continue;
            }
            0xC2..=0xDF => (1usize, 0x80u8, 0xBFu8),
            0xE0 => (2, 0xA0, 0xBF),
            0xE1..=0xEF => (2, 0x80, 0xBF),
            0xF0 => (3, 0x90, 0xBF),
            0xF1..=0xF3 => (3, 0x80, 0xBF),
            0xF4 => (3, 0x80, 0x8F),
            _ => return None,
        };
        let end = width.saturating_add(1);
        let chunk = rest.get(..end)?;
        let (_, continuation) = chunk.split_first()?;
        let mut following = continuation.iter();
        let second = *following.next()?;
        if !(minimum..=maximum).contains(&second) {
            return None;
        }
        if !following.all(|byte| (0x80..=0xBF).contains(byte)) {
            return None;
        }
        rest = rest.get(end..)?;
        decoded.push(NON_ASCII);
    }
    Some(decoded)
}
fn decode_units(bytes: &[u8], width: usize, little_endian: bool) -> Option<Vec<u8>> {
    let mut chunks = bytes.chunks_exact(width);
    if !chunks.remainder().is_empty() {
        return None;
    }
    let mut decoded = Vec::new();
    for chunk in &mut chunks {
        let code_point = match width {
            2 => {
                let [first, second] = <[u8; 2]>::try_from(chunk).ok()?;
                if little_endian {
                    u32::from(u16::from_le_bytes([first, second]))
                } else {
                    u32::from(u16::from_be_bytes([first, second]))
                }
            }
            _ => {
                let [first, second, third, fourth] = <[u8; 4]>::try_from(chunk).ok()?;
                if little_endian {
                    u32::from_le_bytes([first, second, third, fourth])
                } else {
                    u32::from_be_bytes([first, second, third, fourth])
                }
            }
        };
        if code_point > MAX_CODE_POINT {
            return None;
        }
        decoded.push(scalar_to_json_byte(code_point));
    }
    Some(decoded)
}
// the JSON grammar only distinguishes ASCII, so every non-ASCII scalar folds to one placeholder
fn scalar_to_json_byte(code_point: u32) -> u8 {
    match u8::try_from(code_point) {
        Ok(byte) => byte.min(NON_ASCII),
        Err(_) => NON_ASCII,
    }
}
