use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::catalog::{CatalogError, ModelCatalog, UnknownFields};
use crate::fetch_policy::client::{CatalogClient, CatalogRequest};
use crate::fetch_policy::encoding::{DecodeError, decode_json_bytes};
use crate::fetch_policy::integrity::{IntegrityError, IntegrityPolicy, validate_model_cost_map};
use crate::fetch_policy::outcome::{FetchResult, FetchedCatalog, Unavailable};
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource, fetch_with_retry};
use crate::reload::{CatalogService, FetchLimits, ReloadOutcome};
use crate::source::{CatalogProvenance, CatalogSnapshot, SourceKind};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AdoptionError {
    #[error("model cost map from {url} failed integrity validation")]
    Integrity {
        url: String,
        #[source]
        source: IntegrityError,
    },
    #[error("model cost map from {url} is not valid JSON")]
    Encoding {
        url: String,
        #[source]
        source: DecodeError,
    },
    #[error("model cost map from {url} failed to load: {source}")]
    Load {
        url: String,
        #[source]
        source: CatalogError,
    },
}

pub(crate) struct ReloadFetch<'fetch, C, S, J> {
    pub(crate) limits: FetchLimits,
    pub(crate) client: &'fetch C,
    pub(crate) sleeper: &'fetch S,
    pub(crate) jitter: &'fetch mut J,
}

pub(crate) async fn reload_remote<F, C, S, J>(
    service: &CatalogService<F>,
    url: &str,
    fetch: ReloadFetch<'_, C, S, J>,
) -> ReloadOutcome
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
    C: CatalogClient,
    S: CatalogSleeper,
    J: JitterSource,
{
    let request = CatalogRequest {
        url: url.to_owned(),
        timeout: fetch.limits.timeout,
    };
    let result = fetch_with_retry(
        &request,
        fetch.limits.max_attempts,
        fetch.client,
        fetch.sleeper,
        fetch.jitter,
    )
    .await;
    match result {
        FetchResult::Unavailable(unavailable) => {
            tracing::warn!(
                url = %request.url,
                reason = %unavailable.reason,
                "model cost map reload failed; keeping the loaded map"
            );
            ReloadOutcome::Unavailable(unavailable)
        }
        FetchResult::Reloaded(fetched) => adopt_or_keep(service, &request.url, &fetched),
    }
}

fn adopt_or_keep<F>(
    service: &CatalogService<F>,
    url: &str,
    fetched: &FetchedCatalog,
) -> ReloadOutcome
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
{
    match adopt(service, url, fetched, (service.clock)()) {
        Ok(snapshot) => ReloadOutcome::Reloaded(snapshot),
        Err(error) => {
            tracing::warn!(
                url = %url,
                %error,
                "model cost map reload was not adopted; keeping the loaded map"
            );
            ReloadOutcome::Unavailable(Unavailable {
                reason: error.to_string(),
            })
        }
    }
}

pub(crate) fn adopt<F>(
    service: &CatalogService<F>,
    url: &str,
    fetched: &FetchedCatalog,
    loaded_at: DateTime<Utc>,
) -> Result<Arc<CatalogSnapshot>, AdoptionError>
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
{
    let backup = service.bundled_catalog().entries.len();
    validate_model_cost_map(&fetched.models, backup, IntegrityPolicy::default()).map_err(
        |source| AdoptionError::Integrity {
            url: url.to_owned(),
            source,
        },
    )?;
    let text = decode_json_bytes(&fetched.body).map_err(|source| AdoptionError::Encoding {
        url: url.to_owned(),
        source,
    })?;
    let parsed = ModelCatalog::parse(text.as_bytes(), UnknownFields::Warn).map_err(|source| {
        AdoptionError::Load {
            url: url.to_owned(),
            source,
        }
    })?;
    log_unknown_fields(url, &parsed);
    let provenance = CatalogProvenance {
        kind: SourceKind::Remote,
        url: Some(url.to_owned()),
        is_env_forced: false,
        fallback_reason: None,
        loaded_at: Some(loaded_at),
        source_revision: Some(fetched.revision.clone()),
        etag: fetched.etag.clone(),
    };
    let catalog = Arc::new(parsed);
    let snapshot = Arc::new(CatalogSnapshot {
        catalog: Arc::clone(&catalog),
        provenance,
    });
    service.current.store(Arc::clone(&snapshot));
    service.registrations.reapply(&catalog);
    Ok(snapshot)
}

fn log_unknown_fields(url: &str, catalog: &ModelCatalog) {
    for path in &catalog.warnings {
        tracing::warn!(
            url = %url,
            field = %path,
            "unknown field in the fetched model catalog; ignoring it"
        );
    }
}
