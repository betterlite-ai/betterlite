use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::sagemaker_stream::error::SagemakerStreamError;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const END_OF_TEXT: &str = "<|endoftext|>";
pub const STOP_REASON: &str = "stop";

// _chunk_parser_messages_api: the decoder's model fills a chunk that names none, and object is always the chunk one
pub fn chat_chunk(model: &str, value: &Json) -> Result<Chunk, SagemakerStreamError> {
    let Json::Object(mut fields) = value.clone() else {
        return Err(SagemakerStreamError::NotObject);
    };
    fields.insert(
        String::from("object"),
        Json::Text(String::from(CHUNK_OBJECT)),
    );
    if matches!(fields.get("model"), None | Some(Json::Null)) {
        fields.insert(String::from("model"), Json::Text(model.to_owned()));
    }
    let text = serde_json::to_string(&Json::Object(fields)).map_err(SagemakerStreamError::Json)?;
    serde_json::from_str::<Chunk>(&text).map_err(SagemakerStreamError::Json)
}

// _chunk_parser: a missing null token is empty, index counts from zero, and the end token finishes the stream
pub fn token_chunk(value: &Json) -> Result<GenericChunk, SagemakerStreamError> {
    let fields = value.as_object().ok_or(SagemakerStreamError::NotObject)?;
    let token = match fields.get("token") {
        None | Some(Json::Null) => None,
        Some(Json::Object(token)) => Some(token),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(SagemakerStreamError::Field { field: "token" });
        }
    };
    let text = match token.and_then(|token| token.get("text")) {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(SagemakerStreamError::Field {
                field: "token.text",
            });
        }
    };
    let index = match fields.get("index") {
        None | Some(Json::Null) => 0,
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(SagemakerStreamError::Field { field: "index" })?,
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(SagemakerStreamError::Field { field: "index" });
        }
    };
    let finished = text == END_OF_TEXT;
    Ok(GenericChunk {
        text: if finished {
            String::new()
        } else {
            text.to_owned()
        },
        tool_use: None,
        is_finished: finished,
        finish_reason: if finished {
            String::from(STOP_REASON)
        } else {
            String::new()
        },
        usage: None,
        index: Some(index),
        provider_specific_fields: None,
    })
}
mod tests;
