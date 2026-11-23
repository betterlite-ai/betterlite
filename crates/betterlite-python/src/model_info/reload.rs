use super::convert::{self, ReloadEnvelope, ReloadQuery};
use super::error::ModelInfoError;
use super::map::finalize;
use super::state::{Runtime, catalog_settings, local_provenance};
use betterlite_config::loader::startup::process_environment;
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::client::{CatalogClient, CatalogRequest};
use betterlite_models::fetch_policy::integrity::{IntegrityPolicy, validate_model_cost_map};
use betterlite_models::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, FetchedCatalog, classify_fetch_error, classify_fetch_response,
};
use betterlite_models::fetch_policy::retry::{CatalogSleeper, JitterSource, fetch_with_retry};
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::source::{CatalogProvenance, SourceKind};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
// ast-grep-ignore: no-padded-one-line-functions - the environment must be read off the caller's process
pub(crate) async fn reload_blocking(
    runtime: &Runtime,
    query: &ReloadQuery,
) -> Result<String, ModelInfoError> {
    let environment = process_environment();
    reload_blocking_with(runtime, query, &runtime.fetch, &environment).await
}
pub(crate) async fn reload_blocking_with<Client>(
    runtime: &Runtime,
    query: &ReloadQuery,
    client: &Client,
    environment: &BTreeMap<String, String>,
) -> Result<String, ModelInfoError>
where
    Client: CatalogClient,
{
    let loaded_at = Utc::now();
    if let Some(answer) = local_answer(runtime, environment)? {
        return Ok(answer);
    }
    runtime.update(|state| {
        state.provenance.url = Some(query.url.clone());
        state.provenance.is_env_forced = false;
        state.provenance.loaded_at = Some(loaded_at);
    });
    let request = remote_request(query)?;
    let outcome = match client.get_async(&request).await {
        Ok(response) => classify_fetch_response(response, &request.url),
        Err(error) => classify_fetch_error(&error, &request.url),
    };
    match outcome {
        AttemptOutcome::Retryable(retryable) => fallback(
            runtime,
            loaded_at,
            format!("Remote fetch failed: {}", retryable.reason),
        ),
        AttemptOutcome::Unavailable(unavailable) => fallback(
            runtime,
            loaded_at,
            format!("Remote fetch failed: {}", unavailable.reason),
        ),
        AttemptOutcome::Reloaded(fetched) => {
            if !map_is_valid(runtime, &fetched) {
                return fallback(
                    runtime,
                    loaded_at,
                    "Remote data failed integrity validation".to_owned(),
                );
            }
            install_fetched(runtime, query, loaded_at, &fetched)
        }
    }
}
// ast-grep-ignore: no-padded-one-line-functions - the environment must be read off the caller's process
pub(crate) async fn reload_async(
    runtime: &Runtime,
    query: &ReloadQuery,
) -> Result<String, ModelInfoError> {
    let environment = process_environment();
    reload_async_with(runtime, query, &runtime.fetch, &environment, &TokioSleep).await
}
pub(crate) async fn reload_async_with<Client, Sleeper>(
    runtime: &Runtime,
    query: &ReloadQuery,
    client: &Client,
    environment: &BTreeMap<String, String>,
    sleeper: &Sleeper,
) -> Result<String, ModelInfoError>
where
    Client: CatalogClient,
    Sleeper: CatalogSleeper,
{
    if let Some(answer) = local_answer(runtime, environment)? {
        return Ok(answer);
    }
    let request = remote_request(query)?;
    retry_answer(runtime, query, request, client, sleeper).await
}
// the refetch path retries a fetchable failure, then installs the map or answers unavailable.
async fn retry_answer<Client, Sleeper>(
    runtime: &Runtime,
    query: &ReloadQuery,
    request: CatalogRequest,
    client: &Client,
    sleeper: &Sleeper,
) -> Result<String, ModelInfoError>
where
    Client: CatalogClient,
    Sleeper: CatalogSleeper,
{
    let fetched = match fetch_with_retry(
        &request,
        query.max_attempts,
        client,
        sleeper,
        &mut NoJitter,
    )
    .await
    {
        FetchResult::Unavailable(unavailable) => return unavailable_answer(&unavailable.reason),
        FetchResult::Reloaded(fetched) => fetched,
    };
    let loaded_at = Utc::now();
    if !map_is_valid(runtime, &fetched) {
        return unavailable_answer(&format!(
            "model cost map from {} failed integrity validation",
            request.url
        ));
    }
    install_fetched(runtime, query, loaded_at, &fetched)
}
fn map_is_valid(runtime: &Runtime, fetched: &FetchedCatalog) -> bool {
    validate_model_cost_map(
        &fetched.models,
        runtime.bundled.count,
        IntegrityPolicy::default(),
    )
    .is_ok()
}
// the local flag answers the bundled map before any fetch is tried.
fn local_answer(
    runtime: &Runtime,
    environment: &BTreeMap<String, String>,
) -> Result<Option<String>, ModelInfoError> {
    if !catalog_settings(environment)?.local {
        return Ok(None);
    }
    let provenance = local_provenance(runtime.bundled.revision.clone(), true, Utc::now());
    bundled_answer(runtime, provenance).map(Some)
}

fn remote_request(query: &ReloadQuery) -> Result<CatalogRequest, ModelInfoError> {
    Ok(CatalogRequest {
        url: query.url.clone(),
        timeout: query.timeout()?,
    })
}
// the reloaded branch every fetch path shares: install the map and answer with it.
fn install_fetched(
    runtime: &Runtime,
    query: &ReloadQuery,
    loaded_at: DateTime<Utc>,
    fetched: &FetchedCatalog,
) -> Result<String, ModelInfoError> {
    let map = ModelCostMap::parse(&fetched.body)?.entries;
    let finalized = finalize(map)?;
    let rules = Arc::new(CatalogGeneralizations::install(finalized.rules).0);
    let provenance = CatalogProvenance {
        kind: SourceKind::Remote,
        url: Some(query.url.clone()),
        is_env_forced: false,
        fallback_reason: None,
        loaded_at: Some(loaded_at),
        source_revision: Some(fetched.revision.clone()),
        etag: fetched.etag.clone(),
    };
    runtime.update(|state| {
        state.generalizations = Arc::clone(&rules);
        state.provenance = provenance;
    });
    convert::reload_result(&ReloadEnvelope::Reloaded {
        model_cost_map: &finalized.entries,
        revision: Some(&fetched.revision),
        etag: fetched.etag.as_deref(),
    })
}
// ast-grep-ignore: no-one-line-functions - every unavailable answer is built the same way
fn unavailable_answer(reason: &str) -> Result<String, ModelInfoError> {
    convert::reload_result(&ReloadEnvelope::Unavailable { reason })
}
// The bundled map is the answer for env-forced reloads and for every failed remote fetch.
fn bundled_answer(
    runtime: &Runtime,
    provenance: CatalogProvenance,
) -> Result<String, ModelInfoError> {
    let bundled = &runtime.bundled;
    runtime.update(|state| {
        state.generalizations = Arc::clone(&bundled.rules);
        state.provenance = provenance;
    });
    convert::reload_result(&ReloadEnvelope::Reloaded {
        model_cost_map: &bundled.finalized,
        revision: bundled.revision.as_deref(),
        etag: None,
    })
}
fn fallback(
    runtime: &Runtime,
    loaded_at: DateTime<Utc>,
    reason: String,
) -> Result<String, ModelInfoError> {
    let mut provenance = local_provenance(runtime.bundled.revision.clone(), false, loaded_at);
    provenance.url.clone_from(&runtime.state().provenance.url);
    provenance.fallback_reason = Some(reason);
    bundled_answer(runtime, provenance)
}
struct TokioSleep;
impl CatalogSleeper for TokioSleep {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        let delay = Duration::try_from_secs_f64(seconds.to_f64().unwrap_or_default())
            .unwrap_or(Duration::ZERO);
        sleep(delay)
    }
}
struct NoJitter;
impl JitterSource for NoJitter {
    #[cfg_attr(test, mutants::skip)]
    fn unit_interval(&mut self) -> Decimal {
        Decimal::ZERO
    }
}
mod tests;
