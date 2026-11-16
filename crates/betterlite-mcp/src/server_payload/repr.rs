use betterlite_prompts::arguments::json::Json;

mod tests;

// the messages carry Python repr text, so a string is quoted the way repr() quotes it
pub(crate) fn text(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(value.len().saturating_add(2));
    rendered.push(quote);
    for character in value.chars() {
        match escape(character, quote) {
            Some(escaped) => rendered.push_str(&escaped),
            None => rendered.push(character),
        }
    }
    rendered.push(quote);
    rendered
}

fn escape(character: char, quote: char) -> Option<String> {
    match character {
        '\\' => Some("\\\\".to_owned()),
        '\n' => Some("\\n".to_owned()),
        '\r' => Some("\\r".to_owned()),
        '\t' => Some("\\t".to_owned()),
        _ if character == quote => Some(format!("\\{quote}")),
        _ if character.is_control() => Some(format!("\\u{:04x}", u32::from(character))),
        _ => None,
    }
}

pub(crate) fn list(values: &[String]) -> String {
    let rendered: Vec<String> = values.iter().map(|value| text(value)).collect();
    let joined = rendered.join(", ");
    format!("[{joined}]")
}

// Python repr for a parsed JSON value, as the dcr_bridge message embeds its auth_type argument
pub(crate) fn value(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(raw) => text(raw),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(value).collect();
            let joined = rendered.join(", ");
            format!("[{joined}]")
        }
        Json::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(name, item)| format!("{}: {}", text(name), value(item)))
                .collect();
            let joined = rendered.join(", ");
            format!("{{{joined}}}")
        }
    }
}
