use betterlite_models::decisions::Json;

// The type name Python's type() prints for the value LiteLLM puts into its error text.
pub(crate) fn python_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

// Python's str() for the value; a container renders its items with repr(), as str() does for lists and dicts.
pub(crate) fn python_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        container => python_repr(container),
    }
}

fn python_repr(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => python_string_repr(text),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(python_repr).collect();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(entries) => {
            let rendered: Vec<String> = entries
                .iter()
                .map(|(key, item)| format!("{}: {}", python_string_repr(key), python_repr(item)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

// Python repr() quotes with ' unless the text holds one and no ", and escapes the characters it names.
fn python_string_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(quote);
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            '\'' if quote == '\'' => rendered.push_str("\\'"),
            '"' if quote == '"' => rendered.push_str("\\\""),
            plain => rendered.push(plain),
        }
    }
    rendered.push(quote);
    rendered
}
