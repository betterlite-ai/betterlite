use betterlite_core::routes::CallType;

// B207's call types in the order of LiteLLM's chat completions guardrail translation table
pub const CHAT_CALL_TYPES: [CallType; 2] = [CallType::Completion, CallType::Acompletion];

// The two chat entries a second time, so the registry can set its chat-completions flag from one list
pub const CHAT_COMPLETIONS_CALL_TYPES: [CallType; 2] = CHAT_CALL_TYPES;

pub const RESPONSES_CALL_TYPES: [CallType; 2] = [CallType::Responses, CallType::Aresponses];

pub const ANTHROPIC_CALL_TYPES: [CallType; 1] = [CallType::AnthropicMessages];

mod tests;
