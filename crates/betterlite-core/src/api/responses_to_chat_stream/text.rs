use serde_json::Value;

const EXPECTING_VALUE: &str = "Expecting value";
const PROPERTY_NAME: &str = "Expecting property name enclosed in double quotes";

#[must_use]
pub(super) fn repr_value(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => repr_text(text),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(repr_value).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(fields) => {
            let mut entries: Vec<(&String, &Value)> = fields.iter().collect();
            entries.sort_by_key(|(key, _)| u8::from(key.as_str() != "type"));
            let rendered: Vec<String> = entries
                .into_iter()
                .map(|(key, item)| format!("{}: {}", repr_text(key), repr_value(item)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

fn repr_text(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push('\'');
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\'' => rendered.push_str("\\'"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ => rendered.push(character),
        }
    }
    rendered.push('\'');
    rendered
}

// json.loads raises JSONDecodeError, whose text names the line, the column and the character
pub(super) fn scanned_error(text: &str) -> Option<String> {
    let start = first_visible(text, 0);
    let after = first_visible(text, start.saturating_add(1));
    match text.chars().nth(start) {
        Some('{') => match text.chars().nth(after) {
            Some('"' | '}') => None,
            Some(_) | None => Some(positioned(PROPERTY_NAME, text, after)),
        },
        Some('"' | 't' | 'f' | 'n' | '-' | '0'..='9') => None,
        Some('[') => Some(positioned(EXPECTING_VALUE, text, after)),
        Some(_) | None => Some(positioned(EXPECTING_VALUE, text, start)),
    }
}

fn first_visible(text: &str, from: usize) -> usize {
    let mut index = from;
    for character in text.chars().skip(from) {
        if !character.is_whitespace() {
            return index;
        }
        index = index.saturating_add(1);
    }
    index
}

fn positioned(message: &str, text: &str, index: usize) -> String {
    let before = prefix_of(text, index);
    let line = before
        .chars()
        .filter(|character| *character == '\n')
        .count()
        .saturating_add(1);
    let column = before
        .chars()
        .rev()
        .take_while(|character| *character != '\n')
        .count()
        .saturating_add(1);
    format!("{message}: line {line} column {column} (char {index})")
}

fn prefix_of(text: &str, index: usize) -> &str {
    text.char_indices()
        .nth(index)
        .map_or(text, |(at, _)| text.get(..at).unwrap_or(text))
}
