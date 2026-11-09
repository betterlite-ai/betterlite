use super::Parser;

pub(super) fn text(parser: &mut Parser<'_>) -> Option<String> {
    let mut out = one_string(parser, false)?;
    loop {
        parser.skip(parser.depth > 0);
        let next = parser.bytes.get(parser.position).copied();
        if !matches!(next, Some(b'\'' | b'"' | b'u' | b'U' | b'r' | b'R')) {
            return Some(out);
        }
        out.push_str(&one_string(parser, false)?);
    }
}

// Adjacent bytes tokens join the way Python joins them; their value is a placeholder either way.
pub(super) fn bytes(parser: &mut Parser<'_>) -> Option<()> {
    loop {
        let start = parser.position;
        one_string(parser, true)?;
        if !parser.text.get(start..parser.position)?.is_ascii() {
            return None;
        }
        parser.skip(parser.depth > 0);
        match parser.bytes.get(parser.position).copied() {
            Some(b'b' | b'B' | b'r' | b'R') => {}
            Some(_) | None => return Some(()),
        }
    }
}

// A bytes body keeps every escape undecoded.
pub(super) fn one_string(parser: &mut Parser<'_>, force_raw: bool) -> Option<String> {
    let (quote, quote_raw) = prefix(parser)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let raw = quote_raw || force_raw;
    parser.position = parser.position.saturating_add(1);
    let triple = parser.bytes.get(parser.position).copied() == Some(quote)
        && parser.bytes.get(parser.position.saturating_add(1)).copied() == Some(quote);
    if triple {
        parser.position = parser.position.saturating_add(2);
    }
    let mut out = String::new();
    loop {
        let byte = parser.bytes.get(parser.position).copied()?;
        if byte == b'\\' {
            backslash(parser, raw, &mut out)?;
            continue;
        }
        if byte == quote && !triple {
            parser.position = parser.position.saturating_add(1);
            return Some(out);
        }
        let closing = parser.bytes.get(parser.position.saturating_add(1)).copied() == Some(quote)
            && parser.bytes.get(parser.position.saturating_add(2)).copied() == Some(quote);
        if byte == quote && triple && closing {
            parser.position = parser.position.saturating_add(3);
            return Some(out);
        }
        if !triple && (byte == b'\n' || byte == b'\r') {
            return None;
        }
        push_char(parser, &mut out)?;
    }
}

// The r, b and u prefix letters the token carries, and whether it is raw.
fn prefix(parser: &mut Parser<'_>) -> Option<(u8, bool)> {
    let mut byte = parser.bytes.get(parser.position).copied()?;
    let mut raw = false;
    for _ in 0..2_u8 {
        if !matches!(byte, b'r' | b'R' | b'b' | b'B' | b'u' | b'U') {
            break;
        }
        raw |= matches!(byte, b'r' | b'R');
        parser.position = parser.position.saturating_add(1);
        byte = parser.bytes.get(parser.position).copied()?;
    }
    Some((byte, raw))
}

fn backslash(parser: &mut Parser<'_>, raw: bool, out: &mut String) -> Option<()> {
    parser.position = parser.position.saturating_add(1);
    if raw {
        out.push('\\');
        push_char(parser, out)
    } else {
        escape(parser, out)
    }
}

fn escape(parser: &mut Parser<'_>, out: &mut String) -> Option<()> {
    let byte = parser.bytes.get(parser.position).copied()?;
    if let Some(character) = simple_escape(byte) {
        parser.position = parser.position.saturating_add(1);
        out.push(character);
        return Some(());
    }
    if byte == b'\n' || byte == b'\r' {
        parser.position = parser.position.saturating_add(1);
        let continuation = byte == b'\r' && parser.bytes.get(parser.position) == Some(&b'\n');
        parser.position = parser.position.saturating_add(usize::from(continuation));
        return Some(());
    }
    if matches!(byte, b'0'..=b'7' | b'x' | b'u' | b'U') {
        out.push(escaped_char(parser, byte)?);
        return Some(());
    }
    if byte == b'N' {
        // No name table exists here, so a well-formed \N{name} keeps its text.
        let rest = parser
            .bytes
            .get(parser.position.saturating_add(1)..)?
            .strip_prefix(b"{")?;
        let close = rest
            .iter()
            .position(|candidate| *candidate == b'}')
            .filter(|close| *close > 0)?;
        let end = parser.position.saturating_add(close).saturating_add(3);
        out.push_str(parser.text.get(parser.position.saturating_sub(1)..end)?);
        parser.position = end;
        return Some(());
    }
    out.push('\\');
    push_char(parser, out)
}

const fn simple_escape(byte: u8) -> Option<char> {
    match byte {
        b'\\' => Some('\\'),
        b'\'' => Some('\''),
        b'"' => Some('"'),
        b'a' => Some('\x07'),
        b'b' => Some('\x08'),
        b'f' => Some('\x0c'),
        b'n' => Some('\n'),
        b'r' => Some('\r'),
        b't' => Some('\t'),
        b'v' => Some('\x0b'),
        _ => None,
    }
}

fn escaped_char(parser: &mut Parser<'_>, marker: u8) -> Option<char> {
    if matches!(marker, b'x' | b'u' | b'U') {
        parser.position = parser.position.saturating_add(1);
        let width = match marker {
            b'x' => 2_u32,
            b'u' => 4_u32,
            _ => 8_u32,
        };
        let value = digit_run(parser, 16, width, true)?;
        return match char::from_u32(value) {
            None if value <= 0x0010_FFFF => Some('\u{fffd}'),
            character => character,
        };
    }
    let value = digit_run(parser, 8, 3, false)?;
    Some(char::from_u32(value).unwrap_or('\u{fffd}'))
}

fn digit_run(parser: &mut Parser<'_>, radix: u32, width: u32, exact: bool) -> Option<u32> {
    let mut value = 0_u32;
    let mut count = 0_u32;
    while count < width {
        let Some(byte) = parser.bytes.get(parser.position).copied() else {
            break;
        };
        let Some(digit) = char::from(byte).to_digit(radix) else {
            break;
        };
        value = value.checked_mul(radix)?.checked_add(digit)?;
        parser.position = parser.position.saturating_add(1);
        count = count.saturating_add(1);
    }
    if exact && count < width {
        return None;
    }
    Some(value)
}

fn push_char(parser: &mut Parser<'_>, out: &mut String) -> Option<()> {
    let rest = parser.text.get(parser.position..)?;
    let character = rest.chars().next()?;
    out.push(character);
    parser.position = parser.position.saturating_add(character.len_utf8());
    Some(())
}
