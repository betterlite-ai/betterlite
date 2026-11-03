use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use thiserror::Error;
pub mod builders;
pub mod wire;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequiredNullable<T> {
    Null(()),
    Value(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputAnnotation {
    pub r#type: Box<RequiredNullable<String>>,

    pub start_index: Box<RequiredNullable<i64>>,

    pub end_index: Box<RequiredNullable<i64>>,

    pub url: Box<RequiredNullable<String>>,

    pub title: Box<RequiredNullable<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputText {
    pub r#type: Box<RequiredNullable<String>>,

    pub text: Box<RequiredNullable<String>>,

    pub annotations: Box<RequiredNullable<Vec<OutputAnnotation>>>,
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
    pub arguments: Box<RequiredNullable<String>>,

    pub call_id: Box<RequiredNullable<String>>,

    pub name: Box<RequiredNullable<String>>,

    pub r#type: Box<RequiredNullable<String>>,
    pub id: Box<RequiredNullable<String>>,
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
    pub result: Box<RequiredNullable<String>>,
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
    pub code: Box<RequiredNullable<String>>,
    pub container_id: Box<RequiredNullable<String>>,
    pub status: OutputCallStatus,
    pub outputs: Box<RequiredNullable<Vec<OutputCodeInterpreterCallLog>>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("Responses websocket operation failed: {message}")]
pub struct ResponsesShapeError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResponseResult {
    pub id: Box<RequiredNullable<String>>,
    pub object: Box<RequiredNullable<String>>,
    pub deleted: Box<RequiredNullable<bool>>,
    #[serde(skip)]
    _hidden_params: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedResponseId {
    pub custom_llm_provider: Option<String>,
    pub model_id: Option<String>,
    pub response_id: Option<String>,
}

pub trait ResponsesClientWebSocket {
    fn send_text<'a>(
        &'a self,
        data: &'a str,
    ) -> impl Future<Output = Result<(), ResponsesShapeError>> + Send + 'a;
    fn receive_text(&self)
    -> impl Future<Output = Result<String, ResponsesShapeError>> + Send + '_;
}
pub trait ResponsesBackendWebSocket {
    fn recv(
        &self,
        decode: bool,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, ResponsesShapeError>> + Send + '_;
    fn send<'a>(
        &'a self,
        message: &'a str,
    ) -> impl Future<Output = Result<(), ResponsesShapeError>> + Send + 'a;
    fn close(&self) -> impl Future<Output = Result<(), ResponsesShapeError>> + Send + '_;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesWebSocketMessage {
    Text(String),
    Bytes(Vec<u8>),
}
pub trait PresidioGuardrailCallback {
    fn presidio_settings(&self, data: &serde_json::Map<String, Value>) -> Option<Value>;
    fn check_pii<'a>(
        &'a self,
        text: &'a str,
        output_parse_pii: bool,
        presidio_config: Option<&'a Value>,
        request_data: &'a serde_json::Map<String, Value>,
    ) -> impl Future<Output = Result<String, ResponsesShapeError>> + Send + 'a;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsRequestDefaults {
    pub fill_missing: serde_json::Map<String, Value>,
    pub overrides: serde_json::Map<String, Value>,
}

impl WsRequestDefaults {
    pub fn merged_into(
        &self,
        request: &serde_json::Map<String, Value>,
    ) -> serde_json::Map<String, Value> {
        let mut merged = self.fill_missing.clone();
        merged.extend(request.clone());
        merged.extend(self.overrides.clone());
        merged
    }
}

mod tests;
