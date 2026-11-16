use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;

use crate::passthrough_reconstruction::converter::EventChunkConverter;
use crate::passthrough_reconstruction::error::{ConvertError, ReconstructionError};
use crate::passthrough_reconstruction::events::{RawSseChunk, split_sse_chunk_into_events};
use crate::stream_usage_assembly::UsageTokenCounter;
use crate::streamed_response_assembly::cost::{StreamCostCalculator, StreamLogging};
use crate::streamed_response_assembly::records::Chunk;
use crate::streamed_response_assembly::response::Response;
use crate::streamed_response_assembly::{Request, stream_chunk_builder};

pub struct AssemblyRequest<'request, Converter>
where
    Converter: EventChunkConverter,
{
    pub chunks: &'request [RawSseChunk],
    pub converter: &'request mut Converter,
    pub now: i64,
}

pub struct AssemblyServices<'services, L, C>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    pub costs: &'services dyn StreamCostCalculator,
    pub counter: &'services dyn UsageTokenCounter,
    pub logging: Option<&'services dyn StreamLogging>,
    pub hooks: &'services VerboseHooks<L, C>,
}

pub async fn assemble_response_legacy<L, C, Converter>(
    request: AssemblyRequest<'_, Converter>,
    services: &AssemblyServices<'_, L, C>,
) -> Result<Option<Response>, ReconstructionError<Converter::Failure>>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
    Converter: EventChunkConverter + Send,
{
    let chunks = convert_chunks(request.chunks, request.converter, services.hooks)?;
    let assembly = Request {
        chunks: Some(&chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: services.logging,
        costs: services.costs,
        counter: services.counter,
        metadata: None,
        now: request.now,
    };
    Ok(stream_chunk_builder(&assembly, services.hooks).await?)
}

// LiteLLM logs the chunk count before the loop and drops each whole data: [DONE] line before converting.
fn convert_chunks<L, C, Converter>(
    raw_chunks: &[RawSseChunk],
    converter: &mut Converter,
    hooks: &VerboseHooks<L, C>,
) -> Result<Vec<Chunk>, ReconstructionError<Converter::Failure>>
where
    L: LoggerSink,
    C: ConsoleSink,
    Converter: EventChunkConverter,
{
    let count = raw_chunks.len();
    let _logged = hooks.utils_print_verbose(
        Payload::String(format!(
            "Building complete streaming response from {count} chunks"
        )),
        "DEBUG",
        &Payload::Bool(true),
    );
    let mut chunks = Vec::new();
    for raw in raw_chunks {
        for event in split_sse_chunk_into_events(raw)? {
            if event.split('\n').any(|line| line.trim() == "data: [DONE]") {
                continue;
            }
            match converter.convert_event(&event) {
                Ok(chunk) => chunks.extend(chunk),
                Err(ConvertError::Malformed) => log_skipped_event(hooks, &event),
                Err(ConvertError::Ended) => break,
                Err(ConvertError::Failed { source }) => {
                    return Err(ReconstructionError::Conversion { source });
                }
            }
        }
    }
    Ok(chunks)
}

fn log_skipped_event<L, C>(hooks: &VerboseHooks<L, C>, event: &str)
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let preview: String = event.chars().take(200).collect();
    let _logged = hooks.utils_print_verbose(
        Payload::String(format!("Skipping non-JSON SSE event: {preview}")),
        "DEBUG",
        &Payload::Bool(true),
    );
}
