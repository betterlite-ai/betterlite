use serde::{Deserialize, Serialize};

use super::content::{
    CodeExecutionCallArguments, FileSearchResult, GoogleSearchCallArguments, GoogleSearchResult,
    ThoughtSummaryItem, UrlContextCallArguments, UrlContextResult,
};
use super::enums::MediaResolution;
use super::json::{Json, Object};
use super::usage::Annotation;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDeltaValue {
    Text(TextDelta),
    Image(ImageDelta),
    Audio(AudioDelta),
    Document(DocumentDelta),
    Video(VideoDelta),
    ThoughtSummary(ThoughtSummaryDelta),
    ThoughtSignature(ThoughtSignatureDelta),
    FunctionCall(FunctionCallDelta),
    FunctionResult(FunctionResultDelta),
    CodeExecutionCall(CodeExecutionCallDelta),
    CodeExecutionResult(CodeExecutionResultDelta),
    UrlContextCall(UrlContextCallDelta),
    UrlContextResult(UrlContextResultDelta),
    GoogleSearchCall(GoogleSearchCallDelta),
    GoogleSearchResult(GoogleSearchResultDelta),
    McpServerToolCall(McpServerToolCallDelta),
    McpServerToolResult(McpServerToolResultDelta),
    FileSearchResult(FileSearchResultDelta),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TextDelta {
    pub text: Option<String>,
    pub annotations: Option<Vec<Annotation>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImageDelta {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    pub resolution: Option<MediaResolution>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AudioDelta {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DocumentDelta {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoDelta {
    pub data: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    pub resolution: Option<MediaResolution>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ThoughtSummaryDelta {
    pub content: Option<ThoughtSummaryItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ThoughtSignatureDelta {
    pub signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<Object>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FunctionResultDelta {
    pub name: Option<String>,
    pub is_error: Option<bool>,
    pub result: Option<Json>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeExecutionCallDelta {
    pub arguments: Option<CodeExecutionCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeExecutionResultDelta {
    pub result: Option<String>,
    pub is_error: Option<bool>,
    pub signature: Option<String>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextCallDelta {
    pub arguments: Option<UrlContextCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UrlContextResultDelta {
    pub signature: Option<String>,
    pub result: Option<Vec<UrlContextResult>>,
    pub is_error: Option<bool>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchCallDelta {
    pub arguments: Option<GoogleSearchCallArguments>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GoogleSearchResultDelta {
    pub signature: Option<String>,
    pub result: Option<Vec<GoogleSearchResult>>,
    pub is_error: Option<bool>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct McpServerToolCallDelta {
    pub name: Option<String>,
    pub server_name: Option<String>,
    pub arguments: Option<Object>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct McpServerToolResultDelta {
    pub name: Option<String>,
    pub server_name: Option<String>,
    pub result: Option<Json>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileSearchResultDelta {
    pub result: Option<Vec<FileSearchResult>>,
}
