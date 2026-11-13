pub mod callbacks;
pub mod records;
pub mod sse;

use betterlite_core::routes::CallType;

// LiteLLM's guardrail_translation_mappings: the four generateContent call types share this handler
pub const CALL_TYPES: [CallType; 4] = [
    CallType::GenerateContent,
    CallType::AgenerateContent,
    CallType::GenerateContentStream,
    CallType::AgenerateContentStream,
];
