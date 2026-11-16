use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use crate::passthrough_reconstruction::collapse::collapse_pure_text_chunks;
use crate::passthrough_reconstruction::converter::EventChunkConverter;
use crate::passthrough_reconstruction::error::ReconstructionError;
use crate::passthrough_reconstruction::events::RawSseChunk;
use crate::passthrough_reconstruction::legacy::{
    AssemblyRequest, AssemblyServices, assemble_response_legacy,
};
use crate::streamed_response_assembly::response::Response;

pub async fn assemble_response<L, C, Converter>(
    request: AssemblyRequest<'_, Converter>,
    services: &AssemblyServices<'_, L, C>,
) -> Result<Option<Response>, ReconstructionError<Converter::Failure>>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
    Converter: EventChunkConverter + Send,
{
    let collapsed = collapse_pure_text_chunks(request.chunks)?;
    let Some(events) = collapsed else {
        return assemble_response_legacy(request, services).await;
    };
    let chunks: Vec<RawSseChunk> = events.into_iter().map(RawSseChunk::Text).collect();
    let collapsed_request = AssemblyRequest {
        chunks: &chunks,
        converter: request.converter,
        now: request.now,
    };
    assemble_response_legacy(collapsed_request, services).await
}
