use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::decisions::Json;
use crate::fetch_policy::client::{CatalogClient, CatalogRequest, FetchClientError};
use crate::fetch_policy::response::FetchResponse;
use crate::fetch_policy::retry::parse_retry_after_seconds;
use crate::fetch_policy::revision::git_blob_id;

pub const RETRYABLE_FETCH_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchedCatalog {
    pub body: Vec<u8>,
    pub models: IndexMap<String, Json>,
    pub revision: String,
    pub etag: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unavailable {
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Retryable {
    pub reason: String,
    pub retry_after_seconds: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    Reloaded(FetchedCatalog),
    Unavailable(Unavailable),
    Retryable(Retryable),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchResult {
    Reloaded(FetchedCatalog),
    Unavailable(Unavailable),
}

#[must_use]
pub fn classify_fetch_response(response: FetchResponse, url: &str) -> AttemptOutcome {
    if RETRYABLE_FETCH_STATUS_CODES.contains(&response.status) {
        return AttemptOutcome::Retryable(Retryable {
            reason: format!("HTTP {} from {url}", response.status),
            retry_after_seconds: parse_retry_after_seconds(&response),
        });
    }
    if (400..600).contains(&response.status) {
        return AttemptOutcome::Unavailable(Unavailable {
            reason: format!("HTTP {} from {url}", response.status),
        });
    }
    match serde_json::from_slice::<Json>(&response.body) {
        Ok(Json::Object(models)) => AttemptOutcome::Reloaded(FetchedCatalog {
            revision: git_blob_id(&response.body),
            etag: response.headers.get("etag").cloned(),
            body: response.body,
            models,
        }),
        Ok(_) => AttemptOutcome::Unavailable(Unavailable {
            reason: format!("expected a JSON object from {url}"),
        }),
        Err(source) => AttemptOutcome::Unavailable(Unavailable {
            reason: format!("invalid JSON from {url}: {source}"),
        }),
    }
}

#[must_use]
pub fn classify_fetch_error(error: FetchClientError, url: &str) -> AttemptOutcome {
    match error {
        FetchClientError::InvalidUrl { detail } => AttemptOutcome::Unavailable(Unavailable {
            reason: format!("{detail} fetching {url}"),
        }),
        FetchClientError::Transport { detail } => AttemptOutcome::Retryable(Retryable {
            reason: format!("{detail} fetching {url}"),
            retry_after_seconds: None,
        }),
    }
}

pub async fn attempt_fetch<C>(client: &C, request: &CatalogRequest) -> AttemptOutcome
where
    C: CatalogClient,
{
    match client.get_async(request).await {
        Ok(response) => classify_fetch_response(response, &request.url),
        Err(error) => classify_fetch_error(error, &request.url),
    }
}

pub fn attempt_fetch_sync<C>(client: &C, request: &CatalogRequest) -> AttemptOutcome
where
    C: CatalogClient,
{
    match client.get(request) {
        Ok(response) => classify_fetch_response(response, &request.url),
        Err(error) => classify_fetch_error(error, &request.url),
    }
}
