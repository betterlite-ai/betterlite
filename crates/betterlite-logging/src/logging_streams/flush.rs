use std::error::Error;

use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::handler::{SuccessHandler, async_success_handler, success_handler};
use crate::logging_success::types::{SuccessRequest, SuccessResult};

// The provider collector B3572 wraps: it holds the stream bytes and builds the logged response.
pub trait StreamCollector {
    type Error: Error + Send + Sync + 'static;

    fn build_logged_response(
        &self,
        state: &mut LoggingState,
    ) -> Result<Option<SuccessResult>, Self::Error>;
}

// LiteLLM's flush_passthrough_collected_chunks: log the built response once streaming is done.
pub fn flush_passthrough<C>(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    collector: &C,
) -> Result<(), C::Error>
where
    C: StreamCollector + ?Sized,
{
    let Some(result) = collector.build_logged_response(state)? else {
        return Ok(());
    };
    success_handler(handler, state, correlation, &request(result));
    Ok(())
}

// LiteLLM's async_flush_passthrough_collected_chunks.
pub async fn async_flush_passthrough<C>(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    collector: &C,
) -> Result<(), C::Error>
where
    C: StreamCollector + Sync + ?Sized,
{
    let Some(result) = collector.build_logged_response(state)? else {
        return Ok(());
    };
    async_success_handler(handler, state, correlation, &request(result)).await;
    Ok(())
}

// The flush passes no times and no cache hit, so LiteLLM's helper fills in the call start and now.
fn request(result: SuccessResult) -> SuccessRequest {
    SuccessRequest {
        result,
        start_time: None,
        end_time: None,
        cache_hit: None,
        prefer_async_handlers: false,
        standard_logging_object: None,
        guardrail_snapshot: None,
    }
}
