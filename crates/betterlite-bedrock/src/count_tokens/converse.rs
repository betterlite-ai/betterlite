use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::request::{ConverseInput, CountTokensInput, CountTokensRequest, ToolConfig};

#[must_use]
pub fn transform_converse_request(request: &IndexMap<String, Json>) -> CountTokensRequest {
    let messages: &[Json] = match request.get("messages") {
        Some(Json::List(messages)) => messages,
        None | Some(_) => &[],
    };
    let system = transform_system(request.get("system"));
    CountTokensRequest {
        input: CountTokensInput::Converse(ConverseInput {
            messages: messages.iter().map(to_converse_message).collect(),
            system: (!system.is_empty()).then_some(system),
            tool_config: transform_tools(request.get("tools")),
        }),
    }
}

#[must_use]
pub fn to_converse_message(message: &Json) -> Json {
    let (role, content) = match message {
        Json::Object(fields) => (
            fields.get("role").cloned().unwrap_or(Json::Null),
            fields.get("content").cloned(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            (Json::Null, None)
        }
    };
    let blocks = match content.unwrap_or(Json::Text(String::new())) {
        Json::Text(text) => vec![Json::Object(IndexMap::from([(
            "text".to_owned(),
            Json::Text(text),
        )]))],
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Vec::new(),
    };
    Json::Object(IndexMap::from([
        ("role".to_owned(), role),
        ("content".to_owned(), Json::List(blocks)),
    ]))
}

#[must_use]
pub fn transform_system(system: Option<&Json>) -> Vec<Json> {
    match system {
        Some(Json::Text(text)) => vec![Json::Object(IndexMap::from([(
            "text".to_owned(),
            Json::Text(text.clone()),
        )]))],
        Some(Json::List(blocks)) => blocks
            .iter()
            .filter_map(|block| match block {
                Json::Object(fields) => {
                    let text = match fields.get("text") {
                        None | Some(Json::Null) => Json::Text(String::new()),
                        Some(value) => value.clone(),
                    };
                    Some(Json::Object(IndexMap::from([("text".to_owned(), text)])))
                }
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            })
            .collect(),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Vec::new(),
    }
}

#[must_use]
pub fn transform_tools(tools: Option<&Json>) -> Option<ToolConfig> {
    let provided = tools?;
    let supplied = match provided {
        Json::Null | Json::Bool(false) => false,
        Json::Bool(true) => true,
        Json::Number(number) => number.as_f64().is_none_or(|value| value != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    };
    if !supplied {
        return None;
    }
    Some(ToolConfig {
        tools: match provided {
            Json::List(items) => items.iter().map(to_bedrock_tool_spec).collect(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Vec::new()
            }
        },
    })
}

#[must_use]
pub fn sanitize_tool_name(raw: Option<&Json>) -> String {
    let mut name = String::new();
    if let Some(Json::Text(text)) = raw {
        name.extend(text.chars().map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        }));
    }
    let prefixed = match name.chars().next() {
        None => name,
        Some(first) if first.is_ascii_alphabetic() => name,
        Some(_) => format!("t_{name}"),
    };
    prefixed.chars().take(64).collect()
}

#[must_use]
pub fn to_bedrock_tool_spec(tool: &Json) -> Json {
    let empty = IndexMap::new();
    let fields = match tool {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
    };
    let name = sanitize_tool_name(fields.get("name"));
    let description = match fields.get("description") {
        None | Some(Json::Null) => Json::Text(name.clone()),
        Some(value) => value.clone(),
    };
    let input_schema = match fields.get("input_schema") {
        None | Some(Json::Null) => Json::Object(IndexMap::from([
            ("type".to_owned(), Json::Text("object".to_owned())),
            ("properties".to_owned(), Json::Object(IndexMap::new())),
        ])),
        Some(value) => value.clone(),
    };
    Json::Object(IndexMap::from([(
        "toolSpec".to_owned(),
        Json::Object(IndexMap::from([
            ("name".to_owned(), Json::Text(name)),
            ("description".to_owned(), description),
            (
                "inputSchema".to_owned(),
                Json::Object(IndexMap::from([("json".to_owned(), input_schema)])),
            ),
        ])),
    )]))
}
