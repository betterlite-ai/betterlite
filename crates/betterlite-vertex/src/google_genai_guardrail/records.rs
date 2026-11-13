use betterlite_prompts::arguments::json::{Json, Object};

use crate::google_native_content::request::GenerateContentRequest;

// LiteLLM reads wire dicts and google-genai SDK objects alike; betterlite projects the typed B1368 request into the same view
#[must_use]
pub fn project_request(request: &GenerateContentRequest) -> Object {
    let mut object = Object::from([
        ("model".to_owned(), Json::Text(request.model.clone())),
        ("contents".to_owned(), request.contents.clone()),
    ]);
    if !matches!(request.system_instruction, Json::Null) {
        object.insert(
            "systemInstruction".to_owned(),
            request.system_instruction.clone(),
        );
    }
    object
}

// a provider-local text location: the key and index path to the part object whose text is rewritten
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(&'static str),
    Index(usize),
}

pub type Location = Vec<Segment>;

fn field<'value>(container: &'value Json, name: &str) -> Option<&'value Json> {
    let Json::Object(object) = container else {
        return None;
    };
    object.get(name)
}

fn field_mut<'value>(container: &'value mut Json, name: &str) -> Option<&'value mut Json> {
    if let Json::Object(object) = container {
        return object.get_mut(name);
    }
    None
}

fn text_parts(content: &Json) -> Vec<(usize, &str)> {
    let Some(Json::List(parts)) = field(content, "parts") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if let Some(Json::Text(text)) = field(part, "text")
            && !text.is_empty()
        {
            found.push((index, text));
        }
    }
    found
}

// root then config, camelCase then snake_case, and the first non-null value wins even when it holds no text
fn system_instruction(data: &Object) -> Option<(Location, &Json)> {
    let config = data.get("config");
    [
        (
            vec![Segment::Key("systemInstruction")],
            data.get("systemInstruction"),
        ),
        (
            vec![Segment::Key("system_instruction")],
            data.get("system_instruction"),
        ),
        (
            vec![Segment::Key("config"), Segment::Key("systemInstruction")],
            config.and_then(|value| field(value, "systemInstruction")),
        ),
        (
            vec![Segment::Key("config"), Segment::Key("system_instruction")],
            config.and_then(|value| field(value, "system_instruction")),
        ),
    ]
    .into_iter()
    .find(|(_, value)| value.is_some_and(|value| !matches!(*value, Json::Null)))
    .and_then(|(location, value)| value.map(|value| (location, value)))
}

fn append_parts(parts: &mut Vec<(Location, String)>, mut path: Location, content: &Json) {
    path.push(Segment::Key("parts"));
    for (part, text) in text_parts(content) {
        let mut location = path.clone();
        location.push(Segment::Index(part));
        parts.push((location, text.to_owned()));
    }
}

fn request_parts(data: &Object) -> Vec<(Location, String)> {
    let mut parts = Vec::new();
    if let Some((path, instruction)) = system_instruction(data) {
        append_parts(&mut parts, path, instruction);
    }
    match data.get("contents") {
        Some(contents @ Json::Object(_)) => {
            append_parts(&mut parts, vec![Segment::Key("contents")], contents);
        }
        Some(Json::List(items)) => {
            for (content, item) in items.iter().enumerate() {
                append_parts(
                    &mut parts,
                    vec![Segment::Key("contents"), Segment::Index(content)],
                    item,
                );
            }
        }
        _ => {}
    }
    parts
}

#[must_use]
pub fn request_texts(data: &Object) -> (Vec<Location>, Vec<String>) {
    let parts = request_parts(data);
    let locations = parts.iter().map(|(location, _)| location.clone()).collect();
    let texts = parts.into_iter().map(|(_, text)| text).collect();
    (locations, texts)
}

fn response_parts(response: &Object) -> Vec<(Location, String)> {
    let Some(Json::List(candidates)) = response.get("candidates") else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for (candidate, entry) in candidates.iter().enumerate() {
        let Some(content) = field(entry, "content") else {
            continue;
        };
        let path = vec![
            Segment::Key("candidates"),
            Segment::Index(candidate),
            Segment::Key("content"),
        ];
        append_parts(&mut parts, path, content);
    }
    parts
}

#[must_use]
pub fn response_texts(response: &Object) -> (Vec<Location>, Vec<String>) {
    let mut locations = Vec::new();
    let mut texts = Vec::new();
    for (location, text) in response_parts(response) {
        locations.push(location);
        texts.push(text);
    }
    (locations, texts)
}

fn resolve_mut<'data>(data: &'data mut Object, location: &[Segment]) -> Option<&'data mut Json> {
    let (Segment::Key(first), rest) = location.split_first()? else {
        return None;
    };
    let mut current = data.get_mut(*first)?;
    for segment in rest {
        current = match segment {
            Segment::Key(key) => field_mut(current, key)?,
            Segment::Index(index) => match current {
                Json::List(items) => items.get_mut(*index)?,
                _ => return None,
            },
        };
    }
    Some(current)
}

pub fn write_texts(data: &mut Object, locations: &[Location], texts: &[String]) {
    if texts.len() != locations.len() {
        return;
    }
    for (location, text) in locations.iter().zip(texts) {
        if let Some(Json::Object(fields)) = resolve_mut(data, location) {
            fields.insert("text".to_owned(), Json::Text(text.clone()));
        }
    }
}

mod tests;
