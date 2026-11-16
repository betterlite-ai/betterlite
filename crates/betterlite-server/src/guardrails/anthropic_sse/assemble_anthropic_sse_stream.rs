use std::future::Future;

use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_models::decisions::Json;

use super::sse_stream::{anthropic_message_start, joined_sse_stream, parsed_sse_events};

mod tests;

// The Anthropic passthrough handler's assembler: None when the stream does not assemble into a chat response.
pub trait SseAssembler: Sync {
    fn assemble(
        &self,
        sse_stream: &str,
        model: &str,
    ) -> impl Future<Output = Option<ResponseRecord>> + Send;
}

// The assembler does not carry the upstream id and model through, so restore_identity stamps them from message_start; it is off by default to keep the wire shape callers re-emit.
pub async fn assemble_anthropic_sse_stream<A>(
    all_chunks: &[StreamItem],
    assembler: &A,
    restore_identity: bool,
) -> Option<ResponseRecord>
where
    A: SseAssembler,
{
    let sse_stream = joined_sse_stream(all_chunks)?;
    let events = parsed_sse_events(&sse_stream);
    let message_start = anthropic_message_start(&events)?;
    let model = match message_start.get("model") {
        Some(Json::Text(name)) if restore_identity => name.as_str(),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => "",
    };
    let mut assembled = assembler.assemble(&sse_stream, model).await?;
    if !restore_identity {
        return Some(assembled);
    }
    if let Some(Json::Text(message_id)) = message_start.get("id") {
        assembled.id.clone_from(message_id);
    }
    if !model.is_empty() {
        model.clone_into(&mut assembled.model);
    }
    Some(assembled)
}
