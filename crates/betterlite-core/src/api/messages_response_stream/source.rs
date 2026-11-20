use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::api::responses_stream_errors::failure::FailureCause;

// The upstream Responses iterator __anext__ pulls from: the next decoded event, its end, or the exception it raised.
#[async_trait]
pub trait ResponsesEventSource: Send {
    async fn next_event(&mut self) -> Result<Option<Json>, FailureCause>;
}
