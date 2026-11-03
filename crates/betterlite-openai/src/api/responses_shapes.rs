use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
pub mod builders;
pub mod websocket;
pub mod wire;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputAnnotation {
    #[serde(deserialize_with = "Option::deserialize")]
    pub r#type: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub start_index: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub end_index: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub url: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputText {
    #[serde(deserialize_with = "Option::deserialize")]
    pub r#type: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub text: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub annotations: Option<Vec<OutputAnnotation>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsePhase {
    Commentary,
    FinalAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionCallStatus {
    InProgress,
    Completed,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputCallStatus {
    InProgress,
    Completed,
    Incomplete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageGenerationCallType {
    ImageGenerationCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeInterpreterCallType {
    CodeInterpreterCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomToolCallType {
    CustomToolCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeInterpreterLogType {
    Logs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFunctionToolCall {
    #[serde(deserialize_with = "Option::deserialize")]
    pub arguments: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub call_id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub name: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub r#type: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub id: Option<String>,
    pub status: FunctionCallStatus,
    #[serde(default)]
    pub phase: Option<ResponsePhase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericResponseOutputItem {
    pub r#type: String,
    pub id: String,
    pub status: String,
    pub role: String,
    pub content: Vec<OutputText>,
    #[serde(default)]
    pub phase: Option<ResponsePhase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToolCallOutputItem {
    pub r#type: CustomToolCallType,
    pub call_id: String,
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub input: String,
    #[serde(default)]
    pub status: Option<FunctionCallStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputImageGenerationCall {
    pub r#type: ImageGenerationCallType,
    pub id: String,
    pub status: OutputCallStatus,
    #[serde(deserialize_with = "Option::deserialize")]
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputCodeInterpreterCallLog {
    pub r#type: CodeInterpreterLogType,
    pub logs: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputCodeInterpreterCall {
    pub r#type: CodeInterpreterCallType,
    pub id: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub code: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub container_id: Option<String>,
    pub status: OutputCallStatus,
    #[serde(deserialize_with = "Option::deserialize")]
    pub outputs: Option<Vec<OutputCodeInterpreterCallLog>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponseResult {
    #[serde(deserialize_with = "Option::deserialize")]
    pub id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub object: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub deleted: Option<bool>,
    #[serde(skip)]
    _hidden_params: BTreeMap<String, Box<RawValue>>,
}

#[serde_with::serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::option_option,
    reason = "TypedDict fields distinguish absent, null and a string"
)]
pub struct DecodedResponseId {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub custom_llm_provider: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub model_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(deserialize_as = "serde_with::FromInto<String>")]
    pub response_id: Option<String>,
}

mod tests;
