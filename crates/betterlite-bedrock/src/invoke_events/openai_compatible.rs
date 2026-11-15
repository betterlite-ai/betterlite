use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::stream;
use betterlite_openai::chat_choices::wire::StreamReply;
use uuid::Uuid;

use crate::invoke_events::error::InvokeStreamError;

pub(crate) fn chunk_parser(
    chunk: &Json,
    next_id: fn() -> Uuid,
    now: fn() -> i64,
) -> Result<StreamReply, InvokeStreamError> {
    let value =
        serde_json::to_value(chunk).map_err(|source| InvokeStreamError::Encode { source })?;
    stream::chunk_parser(&value, next_id, now).map_err(InvokeStreamError::OpenAi)
}
