use std::borrow::Cow;

use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};

// PyYAML's implicit resolver: exactly these spellings are booleans, and any other casing of the words is text.
fn is_pyyaml_true(text: &str) -> bool {
    text == "true"
        || text == "True"
        || text == "TRUE"
        || text == "yes"
        || text == "Yes"
        || text == "YES"
        || text == "on"
        || text == "On"
        || text == "ON"
}

fn is_pyyaml_false(text: &str) -> bool {
    text == "false"
        || text == "False"
        || text == "FALSE"
        || text == "no"
        || text == "No"
        || text == "NO"
        || text == "off"
        || text == "Off"
        || text == "OFF"
}

fn replacement(text: &str) -> Option<String> {
    if is_pyyaml_true(text) {
        return Some("true".to_owned());
    }
    if is_pyyaml_false(text) {
        return Some("false".to_owned());
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
