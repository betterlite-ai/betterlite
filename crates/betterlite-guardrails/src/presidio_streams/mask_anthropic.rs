use super::assembly::StreamAssembly;
use super::budget::{Retention, event_len};
use super::coalesce::Coalescer;
use super::error::{PresidioStreamError, StreamError};
use super::{StreamEvent, StreamRun};
use crate::presidio_response::Mode;
use crate::presidio_response::model::process_response_for_pii;

// Masks a raw Anthropic SSE stream by assembling it, and hands back the original events when the masking changed nothing
pub async fn mask_anthropic_sse_stream<A>(
    run: &mut StreamRun<'_, A>,
    first_chunk: Vec<u8>,
    rest: &mut Coalescer<'_>,
) -> Result<Vec<StreamEvent>, StreamError<A>>
where
    A: StreamAssembly,
{
    let mut retention = Retention::default();
    retention.accept::<A::Error>(first_chunk.len())?;
    let mut chunks = vec![StreamEvent::Bytes(first_chunk)];
    while let Some(item) = rest.coalesce_first_sse_frame().await {
        let coalesced = item.map_err(|source| PresidioStreamError::Upstream { source })?;
        let event = coalesced.into_event();
        retention.accept::<A::Error>(event_len(&event))?;
        chunks.push(event);
    }
    let raw_chunks: Vec<Vec<u8>> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            StreamEvent::Bytes(raw) => Some(raw.clone()),
            StreamEvent::Chat(_) | StreamEvent::Other(_) => None,
        })
        .collect();
    let Some(mut assembled) = run
        .assembly
        .assemble_anthropic_sse_stream(&raw_chunks)
        .await
    else {
        tracing::warn!(
            "Presidio apply_to_output: raw SSE stream could not be assembled into a response. \
             Output PII masking was skipped for this response."
        );
        return Ok(chunks);
    };
    let original =
        serde_json::to_string(&assembled).map_err(|source| PresidioStreamError::Json { source })?;
    process_response_for_pii(run.context, &mut assembled, run.request_data, Mode::Mask).await?;
    if serde_json::to_string(&assembled).map_err(|source| PresidioStreamError::Json { source })?
        == original
    {
        return Ok(chunks);
    }
    let masked = run
        .assembly
        .anthropic_sse_chunks(&assembled)
        .map_err(|source| PresidioStreamError::Assembly { source })?;
    Ok(masked.into_iter().map(StreamEvent::Bytes).collect())
}
