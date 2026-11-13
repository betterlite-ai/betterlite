use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use betterlite_models::decisions::Json;

pub type JsonFields = IndexMap<String, Json>;

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicCompletionResponse {
    pub content: Vec<Json>,
    #[serde(flatten)]
    pub fields: JsonFields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicToolCall {
    pub id: Option<String>,
    pub r#type: String,
    pub function: AnthropicToolFunction,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicToolFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

pub(crate) fn get<'json>(value: &'json Json, field: &str) -> Option<&'json Json> {
    match value {
        Json::Object(fields) => fields.get(field),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn json_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn convert_tool_block(
    block: &Json,
    index: usize,
) -> Result<AnthropicToolCall, super::ResponseContentError> {
    let id = get(block, "id")
        .and_then(json_text)
        .ok_or(super::ResponseContentError::MissingField("id"))?;
    let name = get(block, "name")
        .and_then(json_text)
        .ok_or(super::ResponseContentError::MissingField("name"))?;
    let input = get(block, "input").ok_or(super::ResponseContentError::MissingField("input"))?;
    let caller = get(block, "caller").cloned();
    Ok(AnthropicToolCall {
        id: Some(id.to_owned()),
        r#type: "function".to_owned(),
        function: AnthropicToolFunction {
            name: Some(name.to_owned()),
            arguments: Some(format_json_argument(input)?),
        },
        index,
        caller,
    })
}

pub(crate) fn format_json_argument(value: &Json) -> Result<String, serde_json::Error> {
    match value {
        Json::List(values) => {
            let encoded_values = values
                .iter()
                .map(format_json_argument)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", encoded_values.join(", ")))
        }
        Json::Object(fields) => {
            let pairs = fields
                .iter()
                .map(|(field, item)| {
                    let encoded_field = serde_json::to_string(field)?;
                    Ok(format!("{encoded_field}: {}", format_json_argument(item)?))
                })
                .collect::<Result<Vec<_>, serde_json::Error>>()?;
            Ok(format!("{{{}}}", pairs.join(", ")))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            serde_json::to_string(value)
        }
    }
}
