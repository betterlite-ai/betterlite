use tokio::sync::mpsc::Receiver;

use super::assembly::StreamAssembly;
use super::error::{PresidioStreamError, StreamError};
use super::mask::stream_apply_output_masking;
use super::unmask::stream_pii_unmasking;
use super::{StreamEvent, StreamRun};
use crate::cato::error::ModelStreamError;
use crate::presidio_actions::request_data::pii_tokens;

// LiteLLM's async_post_call_streaming_iterator_hook: masks the stream, unmasks it, or relays it, and a reader that went away ends the run quietly
pub async fn streaming_iterator_hook<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    match route_stream(run, input).await {
        Err(PresidioStreamError::ConsumerClosed) => Ok(()),
        finished => finished,
    }
}

async fn route_stream<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    if run.context.analyzer.settings.apply_to_output {
        return stream_apply_output_masking(run, input).await;
    }
    let tokens = pii_tokens(run.request_data)?;
    if tokens.is_empty() && !run.request_data.is_empty() {
        tracing::debug!("No pii_tokens in request_data['metadata'] for streaming unmask path");
    }
    if run.context.analyzer.settings.output_parse_pii && !tokens.is_empty() {
        return stream_pii_unmasking(run, input).await;
    }
    while let Some(received) = input.recv().await {
        let event = received.map_err(|source| PresidioStreamError::Upstream { source })?;
        run.out.send(event).await?;
    }
    Ok(())
}
