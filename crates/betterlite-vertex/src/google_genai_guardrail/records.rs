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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemSource {
    RootCamel,
    RootSnake,
    ConfigCamel,
    ConfigSnake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    System { source: SystemSource, part: usize },
    Content { content: usize, part: usize },
    Candidate { candidate: usize, part: usize },
}

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

fn part_text(part: &Json) -> Option<&str> {
    if let Some(Json::Text(text)) = field(part, "text")
        && !text.is_empty()
    {
        return Some(text);
    }
    None
}

fn content_text_parts(content: &Json) -> Vec<(usize, &str)> {
    let Some(Json::List(parts)) = field(content, "parts") else {
        return Vec::new();
    };
    parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| part_text(part).map(|text| (index, text)))
        .collect()
}

// root then config, camelCase then snake_case, and the first non-null value wins even when it holds no text
fn system_instruction(data: &Object) -> Option<(SystemSource, &Json)> {
    let config = data.get("config");
    [
        (SystemSource::RootCamel, data.get("systemInstruction")),
        (SystemSource::RootSnake, data.get("system_instruction")),
        (
            SystemSource::ConfigCamel,
            config.and_then(|value| field(value, "systemInstruction")),
        ),
        (
            SystemSource::ConfigSnake,
            config.and_then(|value| field(value, "system_instruction")),
        ),
    ]
    .into_iter()
    .find(|(_, value)| value.is_some_and(|value| !matches!(*value, Json::Null)))
    .and_then(|(source, value)| value.map(|value| (source, value)))
}

fn push_content(parts: &mut Vec<(Location, String)>, content: usize, item: &Json) {
    for (part, text) in content_text_parts(item) {
        parts.push((Location::Content { content, part }, text.to_owned()));
    }
}

fn request_parts(data: &Object) -> Vec<(Location, String)> {
    let mut parts = Vec::new();
    if let Some((source, instruction)) = system_instruction(data) {
        for (part, text) in content_text_parts(instruction) {
            parts.push((Location::System { source, part }, text.to_owned()));
        }
    }
    match data.get("contents") {
        Some(contents @ Json::Object(_)) => push_content(&mut parts, 0, contents),
        Some(Json::List(items)) => {
            for (content, item) in items.iter().enumerate() {
                push_content(&mut parts, content, item);
            }
        }
        _ => {}
    }
    parts
}

#[must_use]
pub fn request_texts(data: &Object) -> (Vec<Location>, Vec<String>) {
    let mut locations = Vec::new();
    let mut texts = Vec::new();
    for (location, text) in request_parts(data) {
        locations.push(location);
        texts.push(text);
    }
    (locations, texts)
}

#[must_use]
pub fn response_texts(response: &Object) -> (Vec<Location>, Vec<String>) {
    let Some(Json::List(candidates)) = response.get("candidates") else {
        return (Vec::new(), Vec::new());
    };
    let mut parts = Vec::new();
    for (candidate, entry) in candidates.iter().enumerate() {
        if let Some(content) = field(entry, "content") {
            parts.extend(
                content_text_parts(content)
                    .into_iter()
                    .map(|(part, text)| (Location::Candidate { candidate, part }, text.to_owned())),
            );
        }
    }
    parts.into_iter().unzip()
}

fn content_part_mut<'data>(content: &'data mut Json, part: usize) -> Option<&'data mut Json> {
    if let Some(Json::List(parts)) = field_mut(content, "parts") {
        return parts.get_mut(part);
    }
    None
}

fn system_part_mut<'data>(
    data: &'data mut Object,
    source: SystemSource,
    part: usize,
) -> Option<&'data mut Json> {
    let container = match source {
        SystemSource::RootCamel => data.get_mut("systemInstruction"),
        SystemSource::RootSnake => data.get_mut("system_instruction"),
        SystemSource::ConfigCamel => field_mut(data.get_mut("config")?, "systemInstruction"),
        SystemSource::ConfigSnake => field_mut(data.get_mut("config")?, "system_instruction"),
    }?;
    content_part_mut(container, part)
}

fn location_part_mut(data: &mut Object, location: Location) -> Option<&mut Json> {
    match location {
        Location::System { source, part } => system_part_mut(data, source, part),
        Location::Content { content, part } => match data.get_mut("contents") {
            Some(Json::List(items)) => items
                .get_mut(content)
                .and_then(|item| content_part_mut(item, part)),
            Some(item @ Json::Object(_)) if content == 0 => content_part_mut(item, part),
            _ => None,
        },
        Location::Candidate { candidate, part } => {
            let Some(Json::List(candidates)) = data.get_mut("candidates") else {
                return None;
            };
            let entry = candidates.get_mut(candidate)?;
            content_part_mut(field_mut(entry, "content")?, part)
        }
    }
}

fn write_part(part: Option<&mut Json>, text: &str) {
    if let Some(Json::Object(fields)) = part {
        fields.insert("text".to_owned(), Json::Text(text.to_owned()));
    }
}

pub fn write_texts(data: &mut Object, locations: &[Location], texts: &[String]) {
    if texts.is_empty() || texts.len() != locations.len() {
        return;
    }
    for (location, text) in locations.iter().zip(texts) {
        write_part(location_part_mut(data, *location), text);
    }
}

mod tests;
