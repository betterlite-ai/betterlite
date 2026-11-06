use std::fmt::Debug;

use betterlite_http::proxy::limits::read_bounded;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::token_cache::get_access_token;

pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub async fn graph_post<B, R>(
    base: &PurviewGuardrailBase,
    url: &str,
    json_body: &B,
    extra_headers: &HeaderMap,
) -> Result<(R, HeaderMap), PurviewGraphError>
where
    B: Serialize + Sync,
    R: DeserializeOwned + Debug,
{
    let token = get_access_token(base).await?;
    tracing::debug!(url, "Purview Graph POST");
    let mut response = base
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
    let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(|source| PurviewGraphError::Body { source })?;
    let response_json: R = serde_json::from_slice(&body)
        .map_err(|source| PurviewGraphError::ResponseJson { source })?;
    tracing::debug!(?response_json, "Purview Graph response");
    Ok((response_json, response_headers))
}

mod tests;
