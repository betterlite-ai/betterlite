use crate::arguments::json::Json;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProtocolValue {
    #[serde(rename = "null_value")]
    Null,
    #[serde(rename = "number_value")]
    Number,
    #[serde(rename = "string_value")]
    String,
    #[serde(rename = "bool_value")]
    Bool,
    #[serde(rename = "struct_value")]
    Struct,
    #[serde(rename = "list_value")]
    List,
}

// Python's int check swallows bool, so LiteLLM answers number_value where its literal declares bool_value.
#[must_use]
pub const fn infer_protocol_value(value: &Json) -> ProtocolValue {
    match value {
        Json::Null => ProtocolValue::Null,
        Json::Bool(_) => ProtocolValue::Bool,
        Json::Number(_) => ProtocolValue::Number,
        Json::Text(_) => ProtocolValue::String,
        Json::Object(_) => ProtocolValue::Struct,
        Json::List(_) => ProtocolValue::List,
    }
}
