mod escape;

use super::scanner::skip_trivia;
use super::{Cursor, Literal, LiteralError};

#[derive(Clone, Copy)]
enum TextPrefix {
    Text,
    RawText,
    Bytes,
    RawBytes,
}

#[derive(Clone, Copy)]
pub(super) struct TextStart {
    prefix: TextPrefix,
    quote_at: usize,
}

fn text_prefix(letters: &str) -> Option<TextPrefix> {
    match letters.to_lowercase().as_str() {
        "" | "u" => Some(TextPrefix::Text),
        "r" => Some(TextPrefix::RawText),
        "b" => Some(TextPrefix::Bytes),
        "br" | "rb" => Some(TextPrefix::RawBytes),
        _other => None,
    }
}

pub(super) fn text_start(cursor: &Cursor<'_>) -> Option<TextStart> {
    let rest = cursor.text.get(cursor.at..)?;
    let quote = rest.find(['\'', '"'])?;
    let prefix = rest.get(..quote)?;
    Some(TextStart {
        prefix: text_prefix(prefix)?,
        quote_at: cursor.at.saturating_add(quote),
    })
}

// implicit concatenation joins adjacent literals, but only within the expression's brackets
pub(super) fn parse_text(cursor: &mut Cursor<'_>, depth: usize) -> Result<Literal, LiteralError> {
    let Some(mut start) = text_start(cursor) else {
        return Err(LiteralError::Syntax);
    };
    let mut value = String::new();
    let bytes = matches!(start.prefix, TextPrefix::Bytes | TextPrefix::RawBytes);
    // each pass consumes at least one quote, so a bounded loop catches a step that stalls
    for _ in 0..=cursor.bytes.len() {
        let piece = scan_text(cursor, start)?;
        if !bytes {
            value.push_str(&piece);
        }
        if skip_trivia(cursor, depth)? {
            return Ok(text_literal(bytes, value));
        }
        let Some(next) = text_start(cursor) else {
            return Ok(text_literal(bytes, value));
        };
        let next_bytes = matches!(next.prefix, TextPrefix::Bytes | TextPrefix::RawBytes);
        if next_bytes != bytes {
            return Err(LiteralError::Syntax);
        }
        start = next;
    }
    Err(LiteralError::Syntax)
}

fn text_literal(bytes: bool, value: String) -> Literal {
    if bytes {
        return Literal::Bytes;
    }
    Literal::Str(value)
}

fn scan_text(cursor: &mut Cursor<'_>, start: TextStart) -> Result<String, LiteralError> {
    let Some(quote) = cursor.bytes.get(start.quote_at).copied() else {
        return Err(LiteralError::Syntax);
    };
    let triple = triple_quote(cursor, start.quote_at);
    let width = if triple { 3 } else { 1 };
    let mut at = start.quote_at.saturating_add(width);
    let mut value = String::new();
    // each pass consumes at least one character, so a bounded loop catches a step that stalls
    for _ in 0..=cursor.bytes.len() {
        let Some(character) = cursor.character(at) else {
            return Err(LiteralError::Syntax);
        };
        if closes_text(cursor, at, triple, quote) {
            cursor.at = at.saturating_add(width);
            return Ok(value);
        }
        if character == '\\' {
            at = escape::scan_escape(cursor, at, start.prefix, &mut value)?;
            continue;
        }
        if !triple && matches!(character, '\n' | '\r') {
            return Err(LiteralError::Syntax);
        }
        if character == '\r' {
            value.push('\n');
            at = cursor.skip_newline_of(at.saturating_add(1));
            continue;
        }
        if matches!(start.prefix, TextPrefix::Bytes | TextPrefix::RawBytes) && !character.is_ascii()
        {
            return Err(LiteralError::Syntax);
        }
        value.push(character);
        at = at.saturating_add(character.len_utf8());
    }
    Err(LiteralError::Syntax)
}

fn triple_quote(cursor: &Cursor<'_>, quote_at: usize) -> bool {
    let Some(quote) = cursor.bytes.get(quote_at).copied() else {
        return false;
    };
    if cursor.bytes.get(quote_at.saturating_add(1)).copied() != Some(quote) {
        return false;
    }
    cursor.bytes.get(quote_at.saturating_add(2)).copied() == Some(quote)
}

fn closes_text(cursor: &Cursor<'_>, at: usize, triple: bool, quote: u8) -> bool {
    if cursor.bytes.get(at).copied() != Some(quote) {
        return false;
    }
    !triple || triple_quote(cursor, at)
}
