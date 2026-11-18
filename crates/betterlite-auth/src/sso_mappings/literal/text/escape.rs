use super::{Cursor, LiteralError, TextPrefix};

const fn simple_escape(byte: u8) -> Option<char> {
    match byte {
        b'\\' => Some('\\'),
        b'\'' => Some('\''),
        b'"' => Some('"'),
        b'a' => Some('\u{7}'),
        b'b' => Some('\u{8}'),
        b'f' => Some('\u{c}'),
        b'n' => Some('\n'),
        b'r' => Some('\r'),
        b't' => Some('\t'),
        b'v' => Some('\u{b}'),
        _other => None,
    }
}

pub(super) fn scan_escape(
    cursor: &Cursor<'_>,
    at: usize,
    prefix: TextPrefix,
    value: &mut String,
) -> Result<usize, LiteralError> {
    let bytes = matches!(prefix, TextPrefix::Bytes | TextPrefix::RawBytes);
    if !matches!(prefix, TextPrefix::RawText | TextPrefix::RawBytes) {
        return scan_plain_escape(cursor, at, bytes, value);
    }
    let Some(character) = cursor.character(at.saturating_add(1)) else {
        return Err(LiteralError::Syntax);
    };
    if character == '\n' {
        value.push_str("\\\n");
        return Ok(at.saturating_add(2));
    }
    if character == '\r' {
        value.push_str("\\\n");
        return Ok(cursor.skip_newline_of(at.saturating_add(2)));
    }
    // a raw string keeps its backslash, which still hides the quote that follows it
    keep_unknown_escape(cursor, at, bytes, value)
}

fn scan_plain_escape(
    cursor: &Cursor<'_>,
    at: usize,
    bytes: bool,
    value: &mut String,
) -> Result<usize, LiteralError> {
    let Some(next) = cursor.bytes.get(at.saturating_add(1)).copied() else {
        return Err(LiteralError::Syntax);
    };
    let after = at.saturating_add(2);
    // a bytes literal processes \x and keeps \u, \U and \N as the backslash and letter it wrote
    match next {
        b'\n' => Ok(after),
        b'\r' => Ok(cursor.skip_newline_of(after)),
        b'x' => push_hex_escape(cursor, after, 2, value),
        b'u' if !bytes => push_hex_escape(cursor, after, 4, value),
        b'U' if !bytes => push_hex_escape(cursor, after, 8, value),
        b'N' if !bytes => Err(LiteralError::NamedEscape),
        _other => scan_short_escape(cursor, at, next, bytes, value),
    }
}

// the \x, \u and \U escapes read two, four or eight hex digits into one code point
fn push_hex_escape(
    cursor: &Cursor<'_>,
    at: usize,
    width: usize,
    value: &mut String,
) -> Result<usize, LiteralError> {
    let mut code = 0_u32;
    let mut index = 0_usize;
    while index < width {
        let Some(byte) = cursor.bytes.get(at.saturating_add(index)).copied() else {
            return Err(LiteralError::Syntax);
        };
        let Some(digit) = char::from(byte).to_digit(16) else {
            return Err(LiteralError::Syntax);
        };
        code = code.saturating_mul(16).saturating_add(digit);
        index = index.saturating_add(1);
    }
    let Some(character) = char::from_u32(code) else {
        return Err(LiteralError::Syntax);
    };
    value.push(character);
    Ok(at.saturating_add(width))
}

fn scan_short_escape(
    cursor: &Cursor<'_>,
    at: usize,
    next: u8,
    bytes: bool,
    value: &mut String,
) -> Result<usize, LiteralError> {
    if let Some(character) = simple_escape(next) {
        value.push(character);
        return Ok(at.saturating_add(2));
    }
    if !(b'0'..=b'7').contains(&next) {
        return keep_unknown_escape(cursor, at, bytes, value);
    }
    // an octal escape takes up to three digits; a bytes literal keeps only the low byte
    let mut code = 0_u32;
    let mut width = 0_usize;
    while width < 3 {
        let Some(byte) = cursor
            .bytes
            .get(at.saturating_add(1).saturating_add(width))
            .copied()
        else {
            break;
        };
        let Some(digit) = char::from(byte).to_digit(8) else {
            break;
        };
        code = code.saturating_mul(8).saturating_add(digit);
        width = width.saturating_add(1);
    }
    let point = if bytes {
        u32::from(code.to_le_bytes()[0])
    } else {
        code
    };
    let Some(character) = char::from_u32(point) else {
        return Err(LiteralError::Syntax);
    };
    value.push(character);
    Ok(at.saturating_add(1).saturating_add(width))
}

// an escape Python does not know keeps its backslash, with a warning in place of an error
fn keep_unknown_escape(
    cursor: &Cursor<'_>,
    at: usize,
    bytes: bool,
    value: &mut String,
) -> Result<usize, LiteralError> {
    let Some(character) = cursor.character(at.saturating_add(1)) else {
        return Err(LiteralError::Syntax);
    };
    if bytes && !character.is_ascii() {
        return Err(LiteralError::Syntax);
    }
    value.push('\\');
    value.push(character);
    Ok(at.saturating_add(1).saturating_add(character.len_utf8()))
}
