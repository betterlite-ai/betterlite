use unicode_general_category::{GeneralCategory, get_general_category};

mod tests;

const NON_PRINTABLE_CATEGORIES: [GeneralCategory; 7] = [
    GeneralCategory::Control,
    GeneralCategory::Format,
    GeneralCategory::PrivateUse,
    GeneralCategory::Unassigned,
    GeneralCategory::LineSeparator,
    GeneralCategory::ParagraphSeparator,
    GeneralCategory::SpaceSeparator,
];

// Mirrors Python repr() for a str: quote choice, backslash escapes, str.isprintable() exclusions.
pub(crate) fn str_repr(text: &str) -> String {
    let quote = pick_quote(text);
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(quote);
    for character in text.chars() {
        append_escaped(&mut rendered, character, quote);
    }
    rendered.push(quote);
    rendered
}

fn pick_quote(text: &str) -> char {
    if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    }
}

fn append_escaped(rendered: &mut String, character: char, quote: char) {
    let unprintable =
        character != ' ' && NON_PRINTABLE_CATEGORIES.contains(&get_general_category(character));
    if let Some(escape) = named_escape(character, quote) {
        rendered.push_str(escape);
    } else if unprintable {
        rendered.push_str(&hex_escape(character));
    } else {
        rendered.push(character);
    }
}

const fn named_escape(character: char, quote: char) -> Option<&'static str> {
    match character {
        '\\' => Some("\\\\"),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        '\'' if quote == '\'' => Some("\\'"),
        '"' if quote == '"' => Some("\\\""),
        _ => None,
    }
}

fn hex_escape(character: char) -> String {
    let code = u32::from(character);
    if code < 0x100 {
        format!("\\x{code:02x}")
    } else if code < 0x1_0000 {
        format!("\\u{code:04x}")
    } else {
        format!("\\U{code:08x}")
    }
}
