use std::collections::BTreeSet;
use std::future::Future;

use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::streamed_response_assembly::cost::StreamCostCalculator;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};

use super::error::RewriteError;
use super::response::ChatResponse;
use crate::guardrails::chat_guardrail_translation::extraction::json_access::{
    field, integer_value, object_items,
};
use crate::guardrails::chat_guardrail_translation::extraction::narrowed_to_choice::narrowed_to_choice;

mod tests;

fn chunk_records(chunks: &[Object]) -> Result<Vec<Chunk>, RewriteError> {
    chunks
        .iter()
        .map(|chunk| Ok(serde_json::from_str(&serde_json::to_string(chunk)?)?))
        .collect()
}

pub trait ChunkAssembler: Sync {
    fn assemble(
        &self,
        chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send;
}

// B2227's builder with the services it needs; no logging object is passed, so usage and cost come from the chunks and the counter.
pub struct StreamChunkAssembler<'services, L, C> {
    pub costs: &'services dyn StreamCostCalculator,
    pub counter: &'services dyn UsageTokenCounter,
    pub hooks: &'services VerboseHooks<L, C>,
    pub now: i64,
}

impl<L, C> ChunkAssembler for StreamChunkAssembler<'_, L, C>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    fn assemble(
        &self,
        chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send {
        async move {
            let records = chunk_records(chunks)?;
            let request = Request {
                chunks: Some(&records),
                messages: None,
                count_prompt_tokens: None,
                logging: None,
                costs: self.costs,
                counter: self.counter,
                metadata: None,
                now: self.now,
            };
            let built = stream_chunk_builder(&request, self.hooks)
                .await
                .map_err(|source| RewriteError::Assembly {
                    source: Box::new(source),
                })?;
            let Some(Response::Chat(chat)) = built else {
                return Err(RewriteError::NotAChatResponse);
            };
            Ok(serde_json::from_str(&serde_json::to_string(&*chat)?)?)
        }
    }
}

// The builder folds every choice into one, so each choice index is rebuilt alone from every chunk (usage-only chunks included) and the rebuilt choices are stitched back with the index the stream gave them.
pub async fn rebuild_per_choice<A>(
    responses_so_far: &[Object],
    assembler: &A,
) -> Result<ChatResponse, RewriteError>
where
    A: ChunkAssembler,
{
    let choice_indices: BTreeSet<i64> = responses_so_far
        .iter()
        .flat_map(|response| object_items(response, "choices"))
        .map(|choice| integer_value(field(choice, "index")).unwrap_or(0))
        .collect();
    let mut base: Option<ChatResponse> = None;
    let mut stitched = Vec::new();
    for index in choice_indices {
        let narrowed: Vec<Object> = responses_so_far
            .iter()
            .map(|response| narrowed_to_choice(response, index))
            .collect();
        let rebuilt = assembler.assemble(&narrowed).await?;
        let Some(mut choice) = rebuilt.choices.first().cloned() else {
            return Err(RewriteError::NoRebuiltChoice { index });
        };
        choice.index = index;
        stitched.push(choice);
        base.get_or_insert(rebuilt);
    }
    let mut response = base.ok_or(RewriteError::NoStreamChoices)?;
    response.choices = stitched;
    Ok(response)
}
