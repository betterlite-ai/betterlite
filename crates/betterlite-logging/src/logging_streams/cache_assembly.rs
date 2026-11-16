use betterlite_models::decisions::Json;

use super::assembly::StreamAssemblyError;
use crate::logging_success::types::SuccessResult;

// B2227's builder over the cache's chunk buffer, injected so logging never calls streaming itself.
pub trait StreamChunks<Chunk> {
    fn push(&mut self, chunk: &Chunk);

    fn build(&self) -> Result<Option<Json>, StreamAssemblyError>;
}

// LiteLLM's _assemble_complete_response_from_streaming_chunks: the whole response returns, others buffer.
pub fn assemble_complete_stream<Chunk, Chunks>(
    result: &SuccessResult,
    chunk: &Chunk,
    chunks: &mut Chunks,
) -> Option<Json>
where
    Chunks: StreamChunks<Chunk>,
{
    if let SuccessResult::Chat(response) = result {
        return Some(response.clone());
    }
    chunks.push(chunk);
    if !matches!(result, SuccessResult::Chunk(_)) || !has_finish_reason(result) {
        return None;
    }
    match chunks.build() {
        Ok(complete) => complete,
        Err(error) => {
            tracing::warn!(
                %error,
                "stream_chunk_builder raised while assembling the cached streaming response"
            );
            None
        }
    }
}

fn has_finish_reason(result: &SuccessResult) -> bool {
    let Json::Object(fields) = result.response() else {
        return false;
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        return false;
    };
    let Some(Json::Object(first)) = choices.first() else {
        return false;
    };
    match first.get("finish_reason") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            true
        }
    }
}
