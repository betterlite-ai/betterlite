use crate::rubrik::tool_calls::ToolCallSource;
use betterlite_models::decisions::Json;

#[derive(Clone, Debug, Default)]
pub struct GuardrailInputs {
    pub texts: Vec<String>,
    pub tool_calls: Vec<ToolCallSource>,
    pub structured_messages: Vec<Json>,
    pub model: Option<String>,
    pub tools: Option<Json>,
}

// LiteLLM passes only "request" or "response" here; any other value is returned unchanged instead of moderated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModerationInputType {
    Request,
    Response,
    Unrecognized,
}
