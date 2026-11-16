use betterlite_models::decisions::Json;

use crate::logging_state::LoggingState;
use crate::logging_success::ports::StreamAssembler;
use crate::logging_success::types::{SuccessMode, SuccessResult};

pub const TEXT_COMPLETION_OBJECT: &str = "text_completion";

// LiteLLM's _get_assembled_streaming_response gate and chat branch; B9840 adds the other branches.
pub struct ChatStreamAssembler;

impl StreamAssembler for ChatStreamAssembler {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        if state.stream != Some(true) {
            return None;
        }
        match result {
            SuccessResult::Chat(response) => Some(response.clone()),
            SuccessResult::Other(response) if is_text_completion(response) => {
                Some(response.clone())
            }
            SuccessResult::Embeddings(_) | SuccessResult::Chunk(_) | SuccessResult::Other(_) => {
                None
            }
        }
    }
}

fn is_text_completion(response: &Json) -> bool {
    match response {
        Json::Object(fields) => matches!(
            fields.get("object"),
            Some(Json::Text(object)) if object == TEXT_COMPLETION_OBJECT
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}
