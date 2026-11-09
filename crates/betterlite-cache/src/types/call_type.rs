use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CachingSupportedCallType {
    Completion,
    Acompletion,
    Embedding,
    Aembedding,
    Atranscription,
    Transcription,
    AtextCompletion,
    TextCompletion,
    Arerank,
    Rerank,
    Responses,
    Aresponses,
    AnthropicMessages,
    AanthropicMessages,
}

pub const SUPPORTED_CALL_TYPES: [CachingSupportedCallType; 14] = [
    CachingSupportedCallType::Completion,
    CachingSupportedCallType::Acompletion,
    CachingSupportedCallType::Embedding,
    CachingSupportedCallType::Aembedding,
    CachingSupportedCallType::Atranscription,
    CachingSupportedCallType::Transcription,
    CachingSupportedCallType::AtextCompletion,
    CachingSupportedCallType::TextCompletion,
    CachingSupportedCallType::Arerank,
    CachingSupportedCallType::Rerank,
    CachingSupportedCallType::Responses,
    CachingSupportedCallType::Aresponses,
    CachingSupportedCallType::AnthropicMessages,
    CachingSupportedCallType::AanthropicMessages,
];
