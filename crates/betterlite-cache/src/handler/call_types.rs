use crate::cache_key::value::KeyObject;
use crate::types::call_type::CachingSupportedCallType;

use super::HandlerError;
use super::kwargs::no_store_requested;

#[must_use]
pub const fn call_type_name(call_type: CachingSupportedCallType) -> &'static str {
    match call_type {
        CachingSupportedCallType::Completion => "completion",
        CachingSupportedCallType::Acompletion => "acompletion",
        CachingSupportedCallType::Embedding => "embedding",
        CachingSupportedCallType::Aembedding => "aembedding",
        CachingSupportedCallType::Atranscription => "atranscription",
        CachingSupportedCallType::Transcription => "transcription",
        CachingSupportedCallType::AtextCompletion => "atext_completion",
        CachingSupportedCallType::TextCompletion => "text_completion",
        CachingSupportedCallType::Arerank => "arerank",
        CachingSupportedCallType::Rerank => "rerank",
        CachingSupportedCallType::Responses => "responses",
        CachingSupportedCallType::Aresponses => "aresponses",
        CachingSupportedCallType::AnthropicMessages => "anthropic_messages",
        CachingSupportedCallType::AanthropicMessages => "aanthropic_messages",
    }
}

// LiteLLM matches the original function's name against the cache's supported_call_types, and aresponses also covers responses
#[must_use]
pub fn call_type_supported(
    original_function: &str,
    supported_call_types: Option<&[CachingSupportedCallType]>,
) -> bool {
    let Some(supported) = supported_call_types else {
        return false;
    };
    match original_function {
        "aresponses" => supported.iter().any(|call_type| {
            matches!(
                call_type,
                CachingSupportedCallType::Aresponses | CachingSupportedCallType::Responses
            )
        }),
        name => supported
            .iter()
            .any(|call_type| call_type_name(*call_type) == name),
    }
}

pub fn should_store_result_in_cache(
    original_function: &str,
    supported_call_types: Option<&[CachingSupportedCallType]>,
    kwargs: &KeyObject,
) -> Result<bool, HandlerError> {
    if !call_type_supported(original_function, supported_call_types) {
        return Ok(false);
    }
    Ok(!no_store_requested(kwargs)?)
}
