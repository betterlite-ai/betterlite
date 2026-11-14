use betterlite_prompts::arguments::json::Json;

#[must_use]
pub(crate) fn repr_json(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => quoted_string(text),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(repr_json).collect();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(key, value)| format!("{}: {}", quoted_string(key), repr_json(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

#[must_use]
pub(crate) fn quoted_list(names: &[String]) -> String {
    let mut rendered = String::from("[");
    for (index, name) in names.iter().enumerate() {
        if index != 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(&quoted_string(name));
    }
    rendered.push(']');
    rendered
}

#[must_use]
pub(crate) fn quoted_string(text: &str) -> String {
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
            escaped if escaped == quote => {
                rendered.push('\\');
                rendered.push(escaped);
            }
            plain => rendered.push(plain),
        }
    }
    rendered.push(quote);
    rendered
}
