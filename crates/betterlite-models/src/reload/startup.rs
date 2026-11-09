use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::catalog::CatalogError;
use crate::fetch_policy::client::{CatalogClient, CatalogRequest};
use crate::fetch_policy::outcome::{
    AttemptOutcome, FetchedCatalog, Retryable, classify_fetch_error, classify_fetch_response,
};
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource};
use crate::reload::refresh::{self, AdoptionError};
use crate::reload::{CatalogService, FetchLimits, RetryTask, StartupOutcome};
use crate::source::CatalogSnapshot;

pub(crate) struct RetryDeps<C, S, J> {
    pub(crate) client: Arc<C>,
    pub(crate) sleeper: Arc<S>,
    pub(crate) jitter: J,
}

pub(crate) async fn remote<F, C, S, J>(
    service: &CatalogService<F>,
    url: &str,
    limits: FetchLimits,
    deps: RetryDeps<C, S, J>,
) -> StartupOutcome<C, S, J>
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
    C: CatalogClient + Send + Sync + 'static,
    S: CatalogSleeper + 'static,
    J: JitterSource + Send + 'static,
{
    let loaded_at = (service.clock)();
    let request = CatalogRequest {
        url: url.to_owned(),
        timeout: limits.timeout,
    };
    let outcome = match deps.client.get_async(&request).await {
        Ok(response) => classify_fetch_response(response, &request.url),
        Err(error) => classify_fetch_error(&error, &request.url),
    };
    match outcome {
        AttemptOutcome::Reloaded(fetched) => reloaded(service, &request.url, &fetched, loaded_at),
        AttemptOutcome::Unavailable(unavailable) => StartupOutcome {
            snapshot: service.fallback_snapshot(
                &request.url,
                format!("Remote fetch failed: {}", unavailable.reason),
                loaded_at,
            ),
            retry: None,
        },
        AttemptOutcome::Retryable(retryable) => StartupOutcome {
            snapshot: service.fallback_snapshot(
                &request.url,
                format!("Remote fetch failed: {}", retryable.reason),
                loaded_at,
            ),
            retry: retry_task(&request, limits, retryable, deps),
        },
    }
}

fn reloaded<F, C, S, J>(
    service: &CatalogService<F>,
    url: &str,
    fetched: &FetchedCatalog,
    loaded_at: DateTime<Utc>,
) -> StartupOutcome<C, S, J>
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
{
    match refresh::adopt(service, url, fetched, loaded_at) {
        Ok(snapshot) => StartupOutcome {
            snapshot,
            retry: None,
        },
        Err(error) => StartupOutcome {
            snapshot: fallback(service, url, &error, loaded_at),
            retry: None,
        },
    }
}

fn fallback<F>(
    service: &CatalogService<F>,
    url: &str,
    error: &AdoptionError,
    loaded_at: DateTime<Utc>,
) -> Arc<CatalogSnapshot>
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
{
    match error {
        AdoptionError::Integrity { .. } => service.fallback_snapshot(
            url,
            "Remote data failed integrity validation".to_owned(),
            loaded_at,
        ),
        AdoptionError::Encoding { .. } | AdoptionError::Load { .. } => service.fallback_snapshot(
            url,
            format!("Remote data failed to load: {error}"),
            loaded_at,
        ),
    }
}

fn retry_task<C, S, J>(
    request: &CatalogRequest,
    limits: FetchLimits,
    outcome: Retryable,
    deps: RetryDeps<C, S, J>,
) -> Option<RetryTask<C, S, J>>
where
    C: CatalogClient + Send + Sync + 'static,
    S: CatalogSleeper + 'static,
    J: JitterSource + Send + 'static,
{
    (limits.max_attempts > 1).then(|| RetryTask {
        request: request.clone(),
        first_outcome: outcome,
        limits,
        client: deps.client,
        sleeper: deps.sleeper,
        jitter: deps.jitter,
    })
}
