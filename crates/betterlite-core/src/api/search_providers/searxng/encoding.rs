use betterlite_config::sections::settings::JsonValue;
use url::form_urlencoded;

use super::SearxngParams;

pub(super) fn encode_params(params: &SearxngParams) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("q", &params.query);
    serializer.append_pair("format", &params.format);
    if let Some(language) = &params.language {
        serializer.append_pair("language", language);
    }
    for (name, value) in &params.extra {
        serializer.append_pair(name, &wire_text(value));
    }
    let encoded = serializer.finish();
    // Python's quote_plus leaves `~` alone and encodes `*`, the reverse of form_urlencoded
    encoded.replace('*', "%2A").replace("%7E", "~")
}

fn wire_text(value: &JsonValue) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    if let Some(flag) = value.as_bool() {
        return if flag { "True" } else { "False" }.to_owned();
    }
    if value.is_null() {
        return "None".to_owned();
    }
    if let Some(items) = value.as_array() {
        let rendered: Vec<String> = items.iter().map(repr_text).collect();
        return format!("[{}]", rendered.join(", "));
    }
    if let Some(entries) = value.as_object() {
        let rendered: Vec<String> = entries
            .iter()
            .map(|(name, item)| format!("{}: {}", quoted_text(name), repr_text(item)))
            .collect();
        return format!("{{{}}}", rendered.join(", "));
    }
    value.to_string()
}

fn repr_text(value: &JsonValue) -> String {
    let Some(text) = value.as_str() else {
        return wire_text(value);
    };
    quoted_text(text)
}

fn quoted_text(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::new();
    rendered.push(quote);
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            current if current == quote => {
                rendered.push('\\');
                rendered.push(current);
            }
            current => rendered.push(current),
        }
    }
    rendered.push(quote);
    rendered
}
