use reqwest::header::HeaderMap;

use super::analysis::AnalyzePayload;
use super::error::VigilGuardError;
use super::handler_protocol::{AsyncPostHandler, HttpResponse, RequestTimeout};
use super::transient::TransportError;

pub async fn post_with_retry(
    handler: &dyn AsyncPostHandler,
    endpoint: &str,
    headers: &HeaderMap,
    payload: &AnalyzePayload,
    timeout: RequestTimeout,
) -> Result<HttpResponse, VigilGuardError> {
    let mut retry_allowed = true;
    loop {
        let failure = match handler.post(endpoint, headers, payload, timeout).await {
            Ok(response) => {
                if response.status.is_success() {
                    return Ok(response);
                }
                TransportError::HttpStatus {
                    status: response.status,
                }
            }
            Err(error) => error,
        };
        if !retry_allowed || !failure.is_transient() {
            return Err(VigilGuardError::Transport { source: failure });
        }
        retry_allowed = false;
        tracing::debug!(error = %failure, "Vigil Guard transient failure; retrying once");
    }
}
