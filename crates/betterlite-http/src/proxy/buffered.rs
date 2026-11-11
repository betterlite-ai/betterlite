use std::mem;

use reqwest::Request;

use crate::error::HttpError;
use crate::proxy::limits::{ReadError, check_headers, read_bounded};

use super::{ProxiedClient, UpstreamResponse, timed_out};

// Buffers a request the caller built on this client, with the caller's own cap: a management answer is small and the caller knows how small.
pub async fn execute_buffered(
    client: &ProxiedClient,
    request: Request,
    body_limit: usize,
) -> Result<UpstreamResponse, HttpError> {
    let _token = client.tracker.token();
    let mut response = client.execute(request).await.map_err(timed_out)?;
    check_headers(response.headers(), client.limits.header_bytes).map_err(|exceeded| {
        HttpError::TooLarge {
            limit: exceeded.limit,
        }
    })?;
    let status = response.status().as_u16();
    let body = read_bounded(&mut response, body_limit)
        .await
        .map_err(|failure| match failure {
            ReadError::TooLarge { limit } => HttpError::TooLarge { limit },
            ReadError::Failed(source) => timed_out(HttpError::Request(source)),
        })?;
    Ok(UpstreamResponse {
        status,
        headers: mem::take(response.headers_mut()),
        body,
    })
}
