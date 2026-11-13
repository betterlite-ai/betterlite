use betterlite_guardrails::guardrail_message_scope::Object;

pub type TextMapping = (usize, Option<usize>);
pub type ToolCallMapping = (usize, usize);

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ExtractedContent {
    pub texts: Vec<String>,
    pub images: Vec<String>,
    pub tool_calls: Vec<Object>,
    pub text_mappings: Vec<TextMapping>,
    pub tool_call_mappings: Vec<ToolCallMapping>,
}
