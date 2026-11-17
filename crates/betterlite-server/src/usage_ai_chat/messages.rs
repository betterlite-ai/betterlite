use super::model::ModelToolCall;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
    },
    AssistantToolCalls {
        content: Option<String>,
        tool_calls: Vec<ModelToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}
