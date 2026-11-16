use reqwest::blocking::Response as BlockingResponse;

use super::SyncStreamingConfig;
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::{ErrorClasses, Logging, MAX_RESPONSE_BYTES};
use crate::interactions::create::{CreateInteractionError, handle_failure};
use crate::interactions::lifecycle::status_failure;
use crate::interactions::lifecycle::synchronous::finish_blocking;

// LiteLLM's _create_sync_streaming_iterator: the config builds the iterator from the whole buffered body
pub fn create_sync_iterator<C>(
    response: BlockingResponse,
    model: Option<&str>,
    logging_obj: &mut Logging,
    interactions_api_config: &C,
) -> Result<C::SyncStream, CreateInteractionError<C::Error>>
where
    C: SyncStreamingConfig,
{
    if response.status().is_success() {
        let buffered = finish_blocking(response)
            .map_err(|failure| handle_failure(interactions_api_config, failure))?;
        interactions_api_config
            .create_sync_stream_iterator(buffered, model, logging_obj)
            .map_err(CreateInteractionError::Provider)
    } else {
        Err(sync_failure(interactions_api_config, response))
    }
}

// _safe_read_response: a body that cannot be read reads as empty, and the real status still names the error
fn sync_failure<C>(config: &C, response: BlockingResponse) -> CreateInteractionError<C::Error>
where
    C: ErrorClasses,
{
    let status_code = response.status().as_u16();
    let headers = response_headers(response.headers());
    let body = read_body(response, MAX_RESPONSE_BYTES).unwrap_or_default();
    handle_failure(config, status_failure(status_code, headers, &body))
}
