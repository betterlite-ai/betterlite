use serde::{Deserialize, Serialize};

use super::enums::MediaResolution;
use super::json::Object;
use super::usage::Annotation;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text(TextContent),
    Image(ImageContent),
    Audio(AudioContent),
    Document(DocumentContent),
    Video(VideoContent),
    Thought(ThoughtContent),
    FunctionCall(FunctionCallContent),
    FunctionResult(FunctionResultContent),
    CodeExecutionCall(CodeExecutionCallContent),
    CodeExecutionResult(CodeExecutionResultContent),
    UrlContextCall(UrlContextCallContent),
    UrlContextResult(UrlContextResultContent),
    GoogleSearchCall(GoogleSearchCallContent),
    GoogleSearchResult(GoogleSearchResultContent),
    McpServerToolCall(McpServerToolCallContent),
    McpServerToolResult(McpServerToolResultContent),
    FileSearchResult(FileSearchResultContent),
}

pub type ThoughtSummary = Vec<ThoughtSummaryItem>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThoughtSummaryItem {
    Text(TextContent),
    Image(ImageContent),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TextContent {
    pub text: Option<String>,
    pub annotations: Option<Vec<Annotation>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImageContent {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    pub resolution: Option<MediaResolution>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AudioContent {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DocumentContent {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoContent {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    pub resolution: Option<MediaResolution>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ThoughtContent {
    pub signature: Option<String>,
    pub summary: Option<ThoughtSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FunctionCallContent {
    pub name: String,
    pub arguments: Object,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FunctionResultContent {
    pub name: Option<String>,
    pub is_error: Option<bool>,
    pub result: ToolResultValue,
    pub call_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolResultValue {
    Text(String),
    Object(Object),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeExecutionCallContent {
    pub arguments: Option<CodeExecutionCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeExecutionCallArguments {
    pub language: Option<Language>,
    pub code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeExecutionResultContent {
    pub result: Option<String>,
    pub is_error: Option<bool>,
    pub signature: Option<String>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextCallContent {
    pub arguments: Option<UrlContextCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextCallArguments {
    pub urls: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextResultContent {
    pub signature: Option<String>,
    pub result: Option<Vec<UrlContextResult>>,
    pub is_error: Option<bool>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextResult {
    pub url: Option<String>,
    pub status: Option<UrlStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlStatus {
    Success,
    Error,
    Paywall,
    Unsafe,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchCallContent {
    pub arguments: Option<GoogleSearchCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchCallArguments {
    pub queries: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchResultContent {
    pub signature: Option<String>,
    pub result: Option<Vec<GoogleSearchResult>>,
    pub is_error: Option<bool>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchResult {
    pub url: Option<String>,
    pub title: Option<String>,
    pub rendered_content: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct McpServerToolCallContent {
    pub name: String,
    pub server_name: String,
    pub arguments: Object,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct McpServerToolResultContent {
    pub name: Option<String>,
    pub server_name: Option<String>,
    pub result: ToolResultValue,
    pub call_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileSearchResultContent {
    pub result: Option<Vec<FileSearchResult>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileSearchResult {
    pub title: Option<String>,
    pub text: Option<String>,
    pub file_search_store: Option<String>,
}
