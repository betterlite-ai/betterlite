use betterlite_prompts::arguments::json::Object;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::token_cache::get_access_token;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub async fn read_limited_body(
    mut response: reqwest::Response,
) -> Result<Vec<u8>, PurviewGraphError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| PurviewGraphError::Request { source })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(PurviewGraphError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn graph_post<B>(
    base: &PurviewGuardrailBase,
    url: &str,
    json_body: &B,
    extra_headers: &HeaderMap,
) -> Result<(Object, HeaderMap), PurviewGraphError>
where
    B: Serialize + Sync,
{
    let token = get_access_token(base).await?;
    tracing::debug!(url, "Purview Graph POST");
    let response = base
        .client
        .post(url)
        .bearer_auth(token.expose_secret())
        .headers(extra_headers.clone())
        .json(json_body)
        .timeout(base.timeout)
        .send()
        .await
        .map_err(|source| PurviewGraphError::Request { source })?;
    let status = response.status();
    if !status.is_success() {
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return Err(PurviewGraphError::Status {
            status,
            retry_after,
        });
    }
    let response_headers = response.headers().clone();
    let body = read_limited_body(response).await?;
    let response_json: Object = serde_json::from_slice(&body)
        .map_err(|source| PurviewGraphError::ResponseJson { source })?;
    tracing::debug!(?response_json, "Purview Graph response");
    Ok((response_json, response_headers))
}

mod tests;
