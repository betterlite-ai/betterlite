const BEST_WIDTH: usize = 80;
const ROOT_INDENT: usize = 2;
const FOLD_BREAK: &str = "\\\n  ";

const ESCAPES: [(char, &str); 15] = [
    ('\0', "\\0"),
    ('\u{7}', "\\a"),
    ('\u{8}', "\\b"),
    ('\t', "\\t"),
    ('\n', "\\n"),
    ('\u{b}', "\\v"),
    ('\u{c}', "\\f"),
    ('\r', "\\r"),
    ('\u{1b}', "\\e"),
    ('"', "\\\""),
    ('\\', "\\\\"),
    ('\u{85}', "\\N"),
    ('\u{a0}', "\\_"),
    ('\u{2028}', "\\L"),
    ('\u{2029}', "\\P"),
];

fn escape(character: char) -> String {
    if let Some((_, text)) = ESCAPES.iter().find(|(escaped, _)| *escaped == character) {
        return (*text).to_owned();
    }
    let code = u32::from(character);
    if code <= 0xff {
        format!("\\x{code:02X}")
    } else if code <= 0xffff {
        format!("\\u{code:04X}")
    } else {
        format!("\\U{code:08X}")
    }
}

// PyYAML's emitter for yaml.safe_dump(text, default_style='"') of a root scalar, without the trailing newline.
pub(super) fn double_quoted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::from('"');
    let mut column = 1_usize;
    let mut start = 0_usize;
    for end in 0..=chars.len() {
        let character = chars.get(end).copied();
        let needs_escape = character.is_none_or(|candidate| {
            matches!(
                candidate,
                '"' | '\\' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}'
            ) || !(' '..='~').contains(&candidate)
        });
        if needs_escape {
            let pending = chars.get(start..end).unwrap_or_default();
            output.extend(pending);
            column = column.saturating_add(pending.len());
            start = end;
            if let Some(escaped_character) = character {
                let escaped = escape(escaped_character);
                column = column.saturating_add(escaped.len());
                output.push_str(&escaped);
                start = end.saturating_add(1);
            }
        }
        let foldable =
            end.saturating_add(1) < chars.len() && (character == Some(' ') || start >= end);
        if foldable && column.saturating_add(end) > BEST_WIDTH.saturating_add(start) {
            output.extend(chars.get(start..end).unwrap_or_default());
            output.push_str(FOLD_BREAK);
            start = start.max(end);
            column = ROOT_INDENT;
            if chars.get(start) == Some(&' ') {
                output.push('\\');
                column = column.saturating_add(1);
            }
        }
    }
    output.push('"');
    output
}
