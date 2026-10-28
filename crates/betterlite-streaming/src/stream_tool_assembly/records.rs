use std::collections::BTreeMap;

use serde::de::{Deserializer, Error as DeError};
use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AssemblyInput {
    Tool { tool: ToolInput },
    Function { function: FunctionInput },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolInput {
    pub chunks: Vec<Chunk>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionInput {
    pub chunks: Vec<Chunk>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Chunk {
    #[serde(default)]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Choice {
    pub index: Option<u32>,
    #[serde(default)]
    pub delta: Delta,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Delta {
    pub tool_calls: Option<Vec<Option<ToolCall>>>,
    pub function_call: Option<FunctionCall>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCall {
    pub index: Option<u32>,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<FunctionFragment>,
    pub custom: Option<CustomFragment>,
    pub provider_specific_fields: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionFragment {
    pub name: Option<String>,
    pub arguments: Option<JsonValue>,
    pub provider_specific_fields: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CustomFragment {
    pub name: Option<String>,
    pub input: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionCall {
    pub name: Option<String>,
    pub arguments: Option<JsonValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(JsonNumber),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonNumber {
    Integer(i64),
    Float(f64),
}

impl Serialize for JsonNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
        }
    }
}

impl<'de> Deserialize<'de> for JsonNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = serde_json::Number::deserialize(deserializer)?;
        if let Some(integer) = number.as_i64() {
            return Ok(Self::Integer(integer));
        }
        if let Some(float) = number.as_f64() {
            return Ok(Self::Float(float));
        }
        Err(DeError::custom("JSON number cannot be represented"))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionOutput {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomOutput {
    pub name: String,
    pub input: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolOutput {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<BTreeMap<String, JsonValue>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AssemblyOutput {
    Tools(Vec<ToolOutput>),
    Function(FunctionOutput),
}
