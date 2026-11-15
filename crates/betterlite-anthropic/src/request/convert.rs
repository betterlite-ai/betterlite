use betterlite_models::decisions::Json;
use betterlite_openai::base_helpers::{DUMMY_TOOLS, HistoryMessage};
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::message_content::Message;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("a request value cannot be read or written as JSON")]
    Json(#[source] serde_json::Error),
}

fn read<T>(encoded: &str) -> Result<T, ConvertError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(encoded).map_err(ConvertError::Json)
}

fn write<T>(value: &T) -> Result<String, ConvertError>
where
    T: serde::Serialize + ?Sized,
{
    serde_json::to_string(value).map_err(ConvertError::Json)
}

// the message records and wire records differ only in their field shapes, so one JSON pass converts either way
pub(super) fn convert_each<Source, Target>(items: &[Source]) -> Result<Vec<Target>, ConvertError>
where
    Source: serde::Serialize,
    Target: serde::de::DeserializeOwned,
{
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        let encoded = write(item)?;
        converted.push(read(&encoded)?);
    }
    Ok(converted)
}

// a mapped record as the parameters map's JSON tree
pub(super) fn encode<T>(value: &T) -> Result<Json, ConvertError>
where
    T: serde::Serialize + ?Sized,
{
    let encoded = write(value)?;
    let decoded: Json = read(&encoded)?;
    Ok(decoded)
}

// a mapped tool definition as a prompt-tree entry the tool list carries
pub(super) fn encode_prompt<T>(value: &T) -> Result<PromptJson, ConvertError>
where
    T: serde::Serialize + ?Sized,
{
    let encoded = write(value)?;
    let decoded: PromptJson = read(&encoded)?;
    Ok(decoded)
}

// a parameters map entry as the tolerant tool-name wire record it is read into
pub(super) fn to_wire<T>(value: &Json) -> Result<T, ConvertError>
where
    T: serde::de::DeserializeOwned,
{
    let encoded = write(value)?;
    let decoded: T = read(&encoded)?;
    Ok(decoded)
}

pub(super) fn dummy_tools() -> Result<Vec<PromptJson>, ConvertError> {
    let mut tools = Vec::with_capacity(DUMMY_TOOLS.len());
    for tool in &DUMMY_TOOLS {
        let encoded = write(tool)?;
        tools.push(read(&encoded)?);
    }
    Ok(tools)
}

// The parameters map's JSON tree as the prompt tree the prompt primitives read
pub(super) fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), prompt_json(entry)))
                .collect(),
        ),
    }
}

// The prompt tree as the parameters map's JSON tree the request body carries
pub(super) fn params_json(value: &PromptJson) -> Json {
    match value {
        PromptJson::Null => Json::Null,
        PromptJson::Bool(flag) => Json::Bool(*flag),
        PromptJson::Number(number) => Json::Number(number.clone()),
        PromptJson::Text(text) => Json::Text(text.clone()),
        PromptJson::List(items) => Json::List(items.iter().map(params_json).collect()),
        PromptJson::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), params_json(entry)))
                .collect(),
        ),
    }
}

pub(super) fn to_messages(items: &[Json]) -> Result<Vec<Message>, ConvertError> {
    convert_each(items)
}

pub(super) fn from_messages(items: &[Message]) -> Result<Vec<Json>, ConvertError> {
    convert_each(items)
}

pub(super) fn to_prompts(items: &[Message]) -> Result<Vec<PromptJson>, ConvertError> {
    convert_each(items)
}

// B2202's history predicates read the same message fields, so one pass feeds all of them
pub(super) fn to_history(items: &[Json]) -> Result<Vec<HistoryMessage>, ConvertError> {
    convert_each(items)
}
