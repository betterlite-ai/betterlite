use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponseContentError {
    #[error("a tool call in the model response could not be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// What the post-call hook hands over: a chat ModelResponse carries text, every other kind (speech, images) is skipped.
#[derive(Debug, Clone)]
pub enum CallResponse {
    Model(ModelResponse),
    Other,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ModelResponse {
    #[serde_as(as = "DefaultOnNull")]
    pub choices: Vec<ResponseChoice>,
}

// A streamed ModelResponse holds StreamingChoices with a delta, a finished one Choices with a message.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ResponseChoice {
    Streaming(StreamingChoice),
    Complete(CompleteChoice),
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamingChoice {
    pub delta: ResponseDelta,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ResponseDelta {
    #[serde_as(as = "DefaultOnNull")]
    pub content: String,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CompleteChoice {
    #[serde_as(as = "DefaultOnNull")]
    pub message: ResponseMessage,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ResponseMessage {
    #[serde_as(as = "DefaultOnNull")]
    pub content: String,
    pub function_call: Option<FunctionCall>,
    #[serde_as(as = "DefaultOnNull")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionCall {
    pub arguments: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolCall {
    Custom(CustomToolCall),
    Function(FunctionToolCall),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: CustomToolKind,
    pub custom: CustomToolPayload,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub enum CustomToolKind {
    #[default]
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomToolPayload {
    pub name: String,
    pub input: String,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionToolCall {
    pub function: ToolFunction,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub id: String,
    #[serde(rename = "type", default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "function".to_owned()
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolFunction {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub arguments: String,
    #[serde(default)]
    pub name: Option<String>,
}

// LiteLLM's get_content_from_model_response: message text, then the function call and each tool call as JSON, or a streamed delta's text.
pub fn extract_content_from_response(
    response: &CallResponse,
) -> Result<String, ResponseContentError> {
    let chat = match response {
        CallResponse::Model(model) => model,
        CallResponse::Other => {
            tracing::debug!("Model Armor: Skipping non-ModelResponse type");
            return Ok(String::new());
        }
    };
    let mut content = String::new();
    for choice in &chat.choices {
        match choice {
            ResponseChoice::Streaming(streaming) => content.push_str(&streaming.delta.content),
            ResponseChoice::Complete(complete) => push_message(&mut content, &complete.message)?,
        }
    }
    Ok(content)
}

fn push_message(
    content: &mut String,
    message: &ResponseMessage,
) -> Result<(), ResponseContentError> {
    content.push_str(&message.content);
    if let Some(function_call) = &message.function_call {
        let encoded = serde_json::to_string(function_call)
            .map_err(|source| ResponseContentError::Encode { source })?;
        content.push_str(&encoded);
    }
    for tool_call in &message.tool_calls {
        let encoded = serde_json::to_string(tool_call)
            .map_err(|source| ResponseContentError::Encode { source })?;
        content.push_str(&encoded);
    }
    Ok(())
}
