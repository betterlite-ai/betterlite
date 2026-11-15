use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("a request value cannot be read or written as JSON")]
    Json(#[source] serde_json::Error),
}

// the message records and wire records differ only in their field shapes, so one JSON pass converts either way
pub(super) fn convert_each<Source, Target>(items: &[Source]) -> Result<Vec<Target>, ConvertError>
where
    Source: serde::Serialize,
    Target: serde::de::DeserializeOwned,
{
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        let encoded = serde_json::to_string(item).map_err(ConvertError::Json)?;
        converted.push(serde_json::from_str(&encoded).map_err(ConvertError::Json)?);
    }
    Ok(converted)
}

// a mapped record as either JSON tree, the one the caller's variable names
pub(super) fn encode<T, Target>(value: &T) -> Result<Target, ConvertError>
where
    T: serde::Serialize + ?Sized,
    Target: serde::de::DeserializeOwned,
{
    let encoded = serde_json::to_string(value).map_err(ConvertError::Json)?;
    serde_json::from_str(&encoded).map_err(ConvertError::Json)
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
