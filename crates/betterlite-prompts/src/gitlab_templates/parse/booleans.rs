use std::borrow::Cow;

use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};

// PyYAML's implicit resolver: exactly these spellings are booleans, and any other casing of the words is text.
fn pyyaml_value(text: &str) -> Option<bool> {
    if text == "true"
        || text == "True"
        || text == "TRUE"
        || text == "yes"
        || text == "Yes"
        || text == "YES"
        || text == "on"
        || text == "On"
        || text == "ON"
    {
        return Some(true);
    }
    if text == "false"
        || text == "False"
        || text == "FALSE"
        || text == "no"
        || text == "No"
        || text == "NO"
        || text == "off"
        || text == "Off"
        || text == "OFF"
    {
        return Some(false);
    }
    None
}

fn replacement(text: &str) -> Option<String> {
    if let Some(flag) = pyyaml_value(text) {
        let word = if flag { "true" } else { "false" };
        return Some(word.to_owned());
    }
    let lowered = text.to_ascii_lowercase();
    if lowered == "true" || lowered == "false" {
        return Some(format!("\"{text}\""));
    }
    None
}

// serde-saphyr's strict booleans still take any casing of true and false, so the source is rewritten for spans of plain scalars.
pub(super) fn normalize(frontmatter: &str) -> Cow<'_, str> {
    let mut parser = Parser::new_from_str(frontmatter);
    let mut rewrites = Vec::new();
    while let Some(pending) = parser.next_event() {
        let Ok((event, span)) = pending else {
            return Cow::Borrowed(frontmatter);
        };
        let Event::Scalar(text, ScalarStyle::Plain, _, tag) = event else {
            continue;
        };
        if tag.is_some() {
            continue;
        }
        let Some(replacement) = replacement(&text) else {
            continue;
        };
        let (Some(start), Some(end)) = (span.start.byte_offset(), span.end.byte_offset()) else {
            return Cow::Borrowed(frontmatter);
        };
        if frontmatter.get(start..end) != Some(text.as_ref()) {
            continue;
        }
        rewrites.push((start, end, replacement));
    }
    if rewrites.is_empty() {
        return Cow::Borrowed(frontmatter);
    }
    let Some(rewritten) = splice(frontmatter, &rewrites) else {
        return Cow::Borrowed(frontmatter);
    };
    Cow::Owned(rewritten)
}

fn splice(frontmatter: &str, rewrites: &[(usize, usize, String)]) -> Option<String> {
    let mut output = String::with_capacity(frontmatter.len());
    let mut cursor = 0;
    for (start, end, replacement) in rewrites {
        output.push_str(frontmatter.get(cursor..*start)?);
        output.push_str(replacement);
        cursor = *end;
    }
    output.push_str(frontmatter.get(cursor..)?);
    Some(output)
}

mod tests;
