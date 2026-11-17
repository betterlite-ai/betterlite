use super::{Cursor, LiteralError, MAX_LITERAL_DEPTH};

// outside brackets a newline ends the expression, so a value may not continue on the next line
pub(super) fn skip_trivia(cursor: &mut Cursor<'_>, depth: usize) -> Result<bool, LiteralError> {
    let mut crossed_line = false;
    loop {
        match cursor.bytes.get(cursor.at).copied() {
            Some(b' ' | b'\t' | b'\x0c') => cursor.at = cursor.at.saturating_add(1),
            Some(b'\n' | b'\r') => {
                crossed_line = crossed_line || depth == 0;
                cursor.at = cursor.at.saturating_add(1);
            }
            Some(b'#') => skip_comment(cursor),
            Some(b'\\') => skip_continuation(cursor)?,
            _other => return Ok(crossed_line),
        }
    }
}

fn skip_comment(cursor: &mut Cursor<'_>) {
    while let Some(byte) = cursor.bytes.get(cursor.at).copied() {
        if matches!(byte, b'\n' | b'\r') {
            return;
        }
        cursor.at = cursor.at.saturating_add(1);
    }
}

fn skip_continuation(cursor: &mut Cursor<'_>) -> Result<(), LiteralError> {
    let Some(rest) = cursor.text.get(cursor.at.saturating_add(1)..) else {
        return Err(LiteralError::Syntax);
    };
    let Some(next) = rest.chars().next() else {
        return Err(LiteralError::Syntax);
    };
    match next {
        '\n' => {
            cursor.at = cursor.at.saturating_add(2);
            Ok(())
        }
        '\r' => {
            cursor.at = cursor.skip_newline_of(cursor.at.saturating_add(2));
            Ok(())
        }
        _other => Err(LiteralError::Syntax),
    }
}

const fn enter(depth: usize) -> Result<(), LiteralError> {
    if depth >= MAX_LITERAL_DEPTH {
        return Err(LiteralError::TooDeep {
            limit: MAX_LITERAL_DEPTH,
        });
    }
    Ok(())
}

pub(super) fn open(cursor: &mut Cursor<'_>, depth: usize, close: u8) -> Result<bool, LiteralError> {
    enter(depth)?;
    cursor.at = cursor.at.saturating_add(1);
    skip_trivia(cursor, depth.saturating_add(1))?;
    Ok(cursor.take(close))
}

pub(super) fn close_or_comma(
    cursor: &mut Cursor<'_>,
    depth: usize,
    close: u8,
) -> Result<bool, LiteralError> {
    skip_trivia(cursor, depth.saturating_add(1))?;
    let Some(byte) = cursor.bytes.get(cursor.at).copied() else {
        return Err(LiteralError::Syntax);
    };
    if byte == close {
        cursor.at = cursor.at.saturating_add(1);
        return Ok(false);
    }
    if byte != b',' {
        return Err(LiteralError::Syntax);
    }
    cursor.at = cursor.at.saturating_add(1);
    skip_trivia(cursor, depth.saturating_add(1))?;
    if cursor.take(close) {
        return Ok(false);
    }
    Ok(true)
}
