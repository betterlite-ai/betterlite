use super::Parser;

// One string token, joined with the adjacent tokens Python concatenates it with.
pub(super) fn text(parser: &mut Parser<'_>) -> Option<String> {
    let mut out = one_string(parser)?;
    loop {
        parser.skip(parser.depth > 0);
        match parser.bytes.get(parser.position).copied() {
            Some(b'\'' | b'"' | b'u' | b'U' | b'r' | b'R') => out.push_str(&one_string(parser)?),
            Some(_) | None => return Some(out),
        }
    }
}

pub(super) fn one_string(parser: &mut Parser<'_>) -> Option<String> {
    let raw = matches!(
        parser.bytes.get(parser.position).copied(),
        Some(b'r' | b'R')
    );
    if !matches!(parser.bytes.get(parser.position).copied()?, b'\'' | b'"') {
        parser.position = parser.position.saturating_add(1);
    }
    let quote = parser.bytes.get(parser.position).copied()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
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
        if byte == b'\r' && parser.bytes.get(parser.position).copied() == Some(b'\n') {
            parser.position = parser.position.saturating_add(1);
        }
        return Some(());
    }
    if matches!(byte, b'0'..=b'7' | b'x' | b'u' | b'U') {
        out.push(escaped_char(parser, byte)?);
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
        return Some(char::from_u32(value).unwrap_or('\u{fffd}'));
    }
    let value = digit_run(parser, 8, 3, false)?;
    Some(char::from_u32(value).unwrap_or('\u{fffd}'))
}

// At most `width` digits in `radix`; a fixed-width escape refuses a shorter run.
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
