use super::super::usage::Usage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaStreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamDelta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaToolCall {
    pub id: Option<String>,
    pub kind: String,
    pub index: u32,
    pub function: DeltaFunction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFunction {
    pub name: Option<String>,
    pub arguments: String,
}
