use crate::limiter_records::{
    EMBEDDING_API_CALL_TYPES, GOOGLE_GENAI_NATIVE_CALL_TYPES, RERANK_API_CALL_TYPES,
    RESPONSES_API_CALL_TYPES, TEXT_COMPLETION_API_CALL_TYPES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallFamily {
    Google,
    Responses,
    Embedding,
    TextCompletion,
    Rerank,
    Other,
}

#[must_use]
pub fn call_family(call_type: Option<&str>) -> CallFamily {
    match call_type {
        Some(kind) if GOOGLE_GENAI_NATIVE_CALL_TYPES.contains(&kind) => CallFamily::Google,
        Some(kind) if RESPONSES_API_CALL_TYPES.contains(&kind) => CallFamily::Responses,
        Some(kind) if EMBEDDING_API_CALL_TYPES.contains(&kind) => CallFamily::Embedding,
        Some(kind) if TEXT_COMPLETION_API_CALL_TYPES.contains(&kind) => CallFamily::TextCompletion,
        Some(kind) if RERANK_API_CALL_TYPES.contains(&kind) => CallFamily::Rerank,
        Some(_) | None => CallFamily::Other,
    }
}
