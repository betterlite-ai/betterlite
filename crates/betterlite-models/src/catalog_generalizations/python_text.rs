use serde_json::value::RawValue;

// LiteLLM formats the label with %s, so a JSON value prints the way Python's str() shows it
pub(crate) fn python_text(value: Option<&RawValue>) -> String {
    match value.map(RawValue::get) {
        None | Some("null") => "None".to_owned(),
        Some("true") => "True".to_owned(),
        Some("false") => "False".to_owned(),
        Some(text) => serde_json::from_str(text).unwrap_or_else(|_| text.to_owned()),
    }
}

pub(crate) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut repr = String::from(quote);
    for character in text.chars() {
        match character {
            _ if character == quote => repr.extend(['\\', quote]),
            _ if character == '\\' || character.is_control() => {
                repr.extend(character.escape_default())
            }
            _ => repr.push(character),
        }
    }
    repr.push(quote);
    repr
}
