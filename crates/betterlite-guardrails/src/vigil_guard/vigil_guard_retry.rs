use reqwest::header::HeaderMap;

use super::vigil_guard_analysis::AnalyzePayload;
use super::vigil_guard_error::VigilGuardError;
use super::vigil_guard_handler_protocol::{AsyncPostHandler, HttpResponse, RequestTimeout};
use super::vigil_guard_transient::TransportError;

pub async fn post_with_retry<H>(
    handler: &H,
    endpoint: &str,
    headers: &HeaderMap,
    payload: &AnalyzePayload,
    timeout: RequestTimeout,
) -> Result<HttpResponse, VigilGuardError>
where
    H: AsyncPostHandler,
{
    let mut retry_allowed = true;
    loop {
        let failure = match handler.post(endpoint, headers, payload, timeout).await {
            Ok(response) => {
                if !response.status.is_client_error() && !response.status.is_server_error() {
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
