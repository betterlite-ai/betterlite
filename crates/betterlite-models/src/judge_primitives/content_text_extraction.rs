use crate::decisions::Json;

mod tests;

pub fn extract_text_from_content(content: &Json) -> String {
    match content {
        Json::Text(text) => text.clone(),
        Json::List(parts) => parts
            .iter()
            .filter_map(text_part)
            .collect::<Vec<&str>>()
            .join(" "),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => String::new(),
    }
}

// LiteLLM keeps only dict parts whose type is exactly "text", and a part with no text adds nothing
fn text_part(part: &Json) -> Option<&str> {
    let Json::Object(fields) = part else {
        return None;
    };
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "text") {
        return None;
    }
    match fields.get("text") {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
