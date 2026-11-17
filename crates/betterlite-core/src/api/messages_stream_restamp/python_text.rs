pub(super) const fn is_space(character: char) -> bool {
    // CPython's str.strip whitespace, which char::is_whitespace does not match
    matches!(
        character,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

// SSE lines end only at CR, LF or CRLF, so a line's other characters stay inside it
pub(super) fn sse_lines_keepends(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !matches!(character, '\r' | '\n') {
            continue;
        }
        let end = if character == '\r'
            && characters.peek().is_some_and(|(_, next)| *next == '\n')
            && let Some((next_index, next_character)) = characters.next()
        {
            next_index.saturating_add(next_character.len_utf8())
        } else {
            index.saturating_add(character.len_utf8())
        };
        if let Some(line) = text.get(start..end) {
            lines.push(line);
        }
        start = end;
    }
    if let Some(tail) = text.get(start..)
        && !tail.is_empty()
    {
        lines.push(tail);
    }
    lines
}

// Python's utf-8 errors="ignore": invalid sequences are dropped, a truncated one at the end included
pub(super) fn decode_utf8_ignore(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes;
    while !rest.is_empty() {
        match str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let prefix = error.valid_up_to();
                if let Some(valid) = rest
                    .get(..prefix)
                    .and_then(|slice| str::from_utf8(slice).ok())
                {
                    text.push_str(valid);
                }
                let skipped = error
                    .error_len()
                    .unwrap_or_else(|| rest.len().saturating_sub(prefix));
                rest = rest
                    .get(prefix.saturating_add(skipped)..)
                    .unwrap_or_default();
            }
        }
    }
    text
}
