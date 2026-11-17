use super::scanner::{close_or_comma, open, skip_trivia};
use super::{Cursor, Literal, LiteralError};

pub(super) fn parse_brace(cursor: &mut Cursor<'_>, depth: usize) -> Result<Literal, LiteralError> {
    if open(cursor, depth, b'}')? {
        return Ok(Literal::Dict(Vec::new()));
    }
    parse_brace_body(cursor, depth)
}

fn parse_brace_body(cursor: &mut Cursor<'_>, depth: usize) -> Result<Literal, LiteralError> {
    let first = cursor.first_element(depth)?;
    if cursor.bytes.get(cursor.at).copied() == Some(b':') {
        return parse_dict(cursor, depth, first);
    }
    cursor
        .parse_rest(depth, b'}', vec![first])
        .map(Literal::Set)
}

fn parse_dict(
    cursor: &mut Cursor<'_>,
    depth: usize,
    first: Literal,
) -> Result<Literal, LiteralError> {
    let mut entries = Vec::new();
    let mut key = first;
    loop {
        cursor.at = cursor.at.saturating_add(1); // the colon after the key
        skip_trivia(cursor, depth.saturating_add(1))?;
        let value = cursor.parse_value(depth.saturating_add(1))?;
        replace_entry(&mut entries, key, value);
        if !close_or_comma(cursor, depth, b'}')? {
            return Ok(Literal::Dict(entries));
        }
        key = parse_next_key(cursor, depth)?;
    }
}

fn parse_next_key(cursor: &mut Cursor<'_>, depth: usize) -> Result<Literal, LiteralError> {
    let key = cursor.parse_value(depth.saturating_add(1))?;
    expect(cursor, depth, b':')?;
    Ok(key)
}

fn expect(cursor: &mut Cursor<'_>, depth: usize, expected: u8) -> Result<(), LiteralError> {
    skip_trivia(cursor, depth.saturating_add(1))?;
    if cursor.bytes.get(cursor.at).copied() != Some(expected) {
        return Err(LiteralError::Syntax);
    }
    Ok(())
}

// a repeated text key keeps its first position and its last value, as Python's dict does
fn replace_entry(entries: &mut Vec<(Literal, Literal)>, key: Literal, value: Literal) {
    let Literal::Str(text) = &key else {
        entries.push((key, value));
        return;
    };
    let duplicate = entries.iter_mut().find(|entry| match &entry.0 {
        Literal::Str(existing) => existing == text,
        Literal::None
        | Literal::Bool(_)
        | Literal::Number
        | Literal::Ellipsis
        | Literal::Bytes
        | Literal::List(_)
        | Literal::Tuple(_)
        | Literal::Set(_)
        | Literal::Dict(_) => false,
    });
    match duplicate {
        Some(entry) => entry.1 = value,
        None => entries.push((key, value)),
    }
}
