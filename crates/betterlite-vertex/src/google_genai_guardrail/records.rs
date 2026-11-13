use betterlite_prompts::arguments::json::{Json, Object};

use crate::google_native_content::request::GenerateContentRequest;

// LiteLLM reads wire dicts and google-genai SDK objects alike; betterlite projects the typed B1368 request into the same object view
#[must_use]
pub fn project_request(request: &GenerateContentRequest) -> Object {
    let mut object = Object::new();
    object.insert("model".to_owned(), Json::Text(request.model.clone()));
    object.insert("contents".to_owned(), request.contents.clone());
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
pub enum RequestLocation {
    System { source: SystemSource, part: usize },
    Content { content: usize, part: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseLocation {
    Candidate { candidate: usize, part: usize },
}

// LiteLLM's _field: a dict lookup for wire objects, which is what an SDK object projects to here
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

// LiteLLM keeps the parts whose text survives _part_text, and write-back must land on the same part positions
fn content_text_parts(content: &Json) -> Vec<(usize, &Json)> {
    let Some(Json::List(parts)) = field(content, "parts") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if part_text(part).is_some() {
            found.push((index, part));
        }
    }
    found
}

fn content_part<'data>(content: &'data Json, index: usize) -> Option<&'data Json> {
    if let Some(Json::List(parts)) = field(content, "parts") {
        return parts.get(index);
    }
    None
}

fn content_part_mut<'value>(content: &'value mut Json, index: usize) -> Option<&'value mut Json> {
    if let Some(Json::List(parts)) = field_mut(content, "parts") {
        return parts.get_mut(index);
    }
    None
}

// root then config, camelCase then snake_case, and the first non-null value wins even when it holds no text
fn system_instruction(data: &Object) -> Option<(SystemSource, &Json)> {
    for (source, name) in [
        (SystemSource::RootCamel, "systemInstruction"),
        (SystemSource::RootSnake, "system_instruction"),
    ] {
        if let Some(value) = data.get(name)
            && !matches!(*value, Json::Null)
        {
            return Some((source, value));
        }
    }
    let config = data.get("config")?;
    for (source, name) in [
        (SystemSource::ConfigCamel, "systemInstruction"),
        (SystemSource::ConfigSnake, "system_instruction"),
    ] {
        if let Some(value) = field(config, name)
            && !matches!(*value, Json::Null)
        {
            return Some((source, value));
        }
    }
    None
}

fn system_container<'data>(data: &'data Object, source: SystemSource) -> Option<&'data Json> {
    match source {
        SystemSource::RootCamel => data.get("systemInstruction"),
        SystemSource::RootSnake => data.get("system_instruction"),
        SystemSource::ConfigCamel => field(data.get("config")?, "systemInstruction"),
        SystemSource::ConfigSnake => field(data.get("config")?, "system_instruction"),
    }
}

fn system_container_mut<'data>(
    data: &'data mut Object,
    source: SystemSource,
) -> Option<&'data mut Json> {
    match source {
        SystemSource::RootCamel => data.get_mut("systemInstruction"),
        SystemSource::RootSnake => data.get_mut("system_instruction"),
        SystemSource::ConfigCamel => field_mut(data.get_mut("config")?, "systemInstruction"),
        SystemSource::ConfigSnake => field_mut(data.get_mut("config")?, "system_instruction"),
    }
}

// contents is one content object or a list of them, and nothing else LiteLLM can iterate
fn content_item<'data>(data: &'data Object, index: usize) -> Option<&'data Json> {
    match data.get("contents") {
        Some(contents @ Json::Object(_)) if index == 0 => Some(contents),
        Some(Json::List(items)) => items.get(index),
        _ => None,
    }
}

fn content_item_mut<'data>(data: &'data mut Object, index: usize) -> Option<&'data mut Json> {
    match data.get_mut("contents") {
        Some(contents @ Json::Object(_)) if index == 0 => Some(contents),
        Some(Json::List(items)) => items.get_mut(index),
        _ => None,
    }
}

#[must_use]
pub fn request_locations(data: &Object) -> Vec<RequestLocation> {
    let mut locations = Vec::new();
    if let Some((source, instruction)) = system_instruction(data) {
        for (part, _) in content_text_parts(instruction) {
            locations.push(RequestLocation::System { source, part });
        }
    }
    let mut index = 0;
    while let Some(item) = content_item(data, index) {
        for (part, _) in content_text_parts(item) {
            locations.push(RequestLocation::Content {
                content: index,
                part,
            });
        }
        index = index.saturating_add(1);
    }
    locations
}

fn request_part<'data>(data: &'data Object, location: RequestLocation) -> Option<&'data Json> {
    match location {
        RequestLocation::System { source, part } => {
            let instruction = system_container(data, source)?;
            content_part(instruction, part)
        }
        RequestLocation::Content { content, part } => {
            let item = content_item(data, content)?;
            content_part(item, part)
        }
    }
}

#[must_use]
pub fn request_texts(data: &Object) -> Vec<String> {
    let mut texts = Vec::new();
    for location in request_locations(data) {
        if let Some(text) = request_part(data, location).and_then(part_text) {
            texts.push(text.to_owned());
        }
    }
    texts
}

pub fn write_request_texts(data: &mut Object, locations: &[RequestLocation], texts: &[String]) {
    if texts.is_empty() || texts.len() != locations.len() {
        return;
    }
    for (location, text) in locations.iter().zip(texts) {
        let part = match location {
            RequestLocation::System { source, part } => system_container_mut(data, *source)
                .and_then(|instruction| content_part_mut(instruction, *part)),
            RequestLocation::Content { content, part } => {
                content_item_mut(data, *content).and_then(|item| content_part_mut(item, *part))
            }
        };
        if let Some(Json::Object(object)) = part {
            object.insert("text".to_owned(), Json::Text(text.clone()));
        }
    }
}

#[must_use]
pub fn response_locations(response: &Object) -> Vec<ResponseLocation> {
    let Some(Json::List(candidates)) = response.get("candidates") else {
        return Vec::new();
    };
    let mut locations = Vec::new();
    for (candidate, entry) in candidates.iter().enumerate() {
        let Some(content) = field(entry, "content") else {
            continue;
        };
        for (part, _) in content_text_parts(content) {
            locations.push(ResponseLocation::Candidate { candidate, part });
        }
    }
    locations
}

fn response_part<'data>(
    response: &'data Object,
    location: ResponseLocation,
) -> Option<&'data Json> {
    let ResponseLocation::Candidate { candidate, part } = location;
    let Some(Json::List(candidates)) = response.get("candidates") else {
        return None;
    };
    let entry = candidates.get(candidate)?;
    content_part(field(entry, "content")?, part)
}

#[must_use]
pub fn response_texts(response: &Object) -> Vec<String> {
    let mut texts = Vec::new();
    for location in response_locations(response) {
        if let Some(text) = response_part(response, location).and_then(part_text) {
            texts.push(text.to_owned());
        }
    }
    texts
}

pub fn write_response_texts(
    response: &mut Object,
    locations: &[ResponseLocation],
    texts: &[String],
) {
    if texts.is_empty() || texts.len() != locations.len() {
        return;
    }
    for (location, text) in locations.iter().zip(texts) {
        let ResponseLocation::Candidate { candidate, part } = *location;
        let Some(Json::List(candidates)) = response.get_mut("candidates") else {
            continue;
        };
        let Some(entry) = candidates.get_mut(candidate) else {
            continue;
        };
        let Some(content) = field_mut(entry, "content") else {
            continue;
        };
        let Some(Json::Object(object)) = content_part_mut(content, part) else {
            continue;
        };
        object.insert("text".to_owned(), Json::Text(text.clone()));
    }
}

mod tests;
