use serde_json::{Map, Value};

const DATA: &str = "data";
const MEDIA_TYPE: &str = "media_type";
const SOURCE: &str = "source";
const TYPE: &str = "type";
const URL: &str = "url";

#[must_use]
pub fn image_sources(block: &Map<String, Value>) -> Vec<String> {
    let Some(source) = block.get(SOURCE).and_then(Value::as_object) else {
        return Vec::new();
    };
    if source.get(TYPE).and_then(Value::as_str) == Some(URL) {
        return match source.get(URL).and_then(Value::as_str) {
            Some(url) if !url.is_empty() => vec![url.to_owned()],
            Some(_) | None => Vec::new(),
        };
    }
    let Some(data) = source
        .get(DATA)
        .and_then(Value::as_str)
        .filter(|candidate| !candidate.is_empty())
    else {
        return Vec::new();
    };
    match source
        .get(MEDIA_TYPE)
        .and_then(Value::as_str)
        .filter(|media_type| !media_type.is_empty())
    {
        Some(media_type) => vec![format!("data:{media_type};base64,{data}")],
        None => vec![data.to_owned()],
    }
}
