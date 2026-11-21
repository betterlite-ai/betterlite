use std::collections::BTreeMap;
use std::future::Future;

use aggregate::{Aggregates, aggregate_entry, endpoint_error, endpoint_models, first_error};
use betterlite_db::health_check_rows::{
    HealthCheckRow, HealthCheckRows, HealthCheckRowsError, NewHealthCheck,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::future::join_all;
use indexmap::IndexMap;
use writes::{elapsed_ms, endpoint_status, latest_by_key};

use crate::model_deprecation::clock::UtcClock;

// B401's deployment type, named here so callers outside the router's wrapper crates can build one
pub type Deployment = DeploymentSpec;

const MAX_ERROR_CHARS: usize = 500;
// _should_persist_health_check_result's one-hour refresh: the comparison is >=, not >
const REFRESH_INTERVAL: TimeDelta = TimeDelta::seconds(3600);

// one (model_name, model_id) pair a litellm_params.model string resolves to
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelParam {
    pub model_name: String,
    pub model_id: Option<String>,
}

// _build_model_param_to_info_mapping's dict: the model parameter to every model listed under it, in list order
pub type ModelParamIndex = IndexMap<String, Vec<ModelParam>>;

// one _aggregate_health_check_results value: a model's counts for this cycle
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregatedHealthResult {
    pub model_name: String,
    pub model_id: Option<String>,
    pub healthy_count: i32,
    pub unhealthy_count: i32,
    pub error_message: Option<String>,
}

// the deployments and the endpoint results of one health cycle, as _save_background_health_checks_to_db receives them
pub struct HealthCycle<'cycle> {
    pub deployments: &'cycle [Deployment],
    pub healthy: &'cycle [JsonObject],
    pub unhealthy: &'cycle [JsonObject],
}

// when the cycle started and who ran it; both callers of the writers pass the pair through unchanged
pub struct SaveRequest<'save> {
    pub start_time: DateTime<Utc>,
    pub checked_by: Option<&'save str>,
}

// _save_health_check_to_db's arguments: one model's endpoint lists from the /health route
pub struct SingleHealthCheck<'check> {
    pub model_name: &'check str,
    pub model_id: Option<&'check str>,
    pub healthy: &'check [JsonObject],
    pub unhealthy: &'check [JsonObject],
    pub start_time: DateTime<Utc>,
    pub user_id: Option<&'check str>,
}

// the latest row per lookup key, keyed as _save_background_health_checks_to_db keys its map
pub type LatestChecks = BTreeMap<String, HealthCheckRow>;

// the two health check table calls the persistence makes; the port lets a test fake record them
pub trait HealthCheckStore: Send + Sync {
    fn stored_checks(
        &self,
    ) -> impl Future<Output = Result<Vec<HealthCheckRow>, HealthCheckRowsError>> + Send;

    fn persist_result(
        &self,
        new: &NewHealthCheck,
    ) -> impl Future<Output = Option<HealthCheckRow>> + Send;
}

impl HealthCheckStore for HealthCheckRows {
    fn stored_checks(
        &self,
    ) -> impl Future<Output = Result<Vec<HealthCheckRow>, HealthCheckRowsError>> + Send {
        Self::query_latest(self)
    }

    fn persist_result(
        &self,
        new: &NewHealthCheck,
    ) -> impl Future<Output = Option<HealthCheckRow>> + Send {
        Self::save_result(self, new)
    }
}

// _build_model_param_to_info_mapping: a deployment without a model string or without a model_name maps to nothing
#[must_use]
pub fn model_param_index(deployments: &[Deployment]) -> ModelParamIndex {
    let mut index = ModelParamIndex::new();
    for deployment in deployments {
        let Some(Json::Text(param)) = deployment.litellm_params.get("model") else {
            continue;
        };
        if param.is_empty() || deployment.model_name.is_empty() {
            continue;
        }
        let model_id = match deployment.model_info.get("id") {
            Some(Json::Text(id)) if !id.is_empty() => Some(id.clone()),
            Some(
                Json::Text(_)
                | Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => None,
        };
        index.entry(param.clone()).or_default().push(ModelParam {
            model_name: deployment.model_name.clone(),
            model_id,
        });
    }
    index
}

// _aggregate_health_check_results: the healthy pass first, then the keys only the unhealthy pass adds
#[must_use]
pub fn aggregate_results(
    index: &ModelParamIndex,
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
) -> Vec<AggregatedHealthResult> {
    let mut aggregates = Aggregates::new();
    for model in healthy
        .iter()
        .flat_map(|endpoint| endpoint_models(endpoint, index))
    {
        let entry = aggregate_entry(&mut aggregates, model);
        entry.healthy_count = entry.healthy_count.saturating_add(1_i32);
    }
    // every unhealthy endpoint raises the count, but only the first non-empty error is kept for the key
    for endpoint in unhealthy {
        let error = endpoint_error(endpoint)
            .map(|text| text.chars().take(MAX_ERROR_CHARS).collect::<String>());
        for model in endpoint_models(endpoint, index) {
            let entry = aggregate_entry(&mut aggregates, model);
            entry.unhealthy_count = entry.unhealthy_count.saturating_add(1_i32);
            if entry.error_message.is_none() {
                entry.error_message.clone_from(&error);
            }
        }
    }
    aggregates.into_values().collect()
}

// _new_health_status: any healthy endpoint makes the model healthy
#[must_use]
pub const fn new_status(result: &AggregatedHealthResult) -> &'static str {
    if result.healthy_count > 0_i32 {
        return "healthy";
    }
    "unhealthy"
}

// _should_persist_health_check_result: no previous row, a changed status, or a row at least an hour old
#[must_use]
pub fn should_persist(
    result: &AggregatedHealthResult,
    latest: &LatestChecks,
    now: DateTime<Utc>,
) -> bool {
    let lookup_key = match &result.model_id {
        Some(id) if !id.is_empty() => id.as_str(),
        Some(_) | None => result.model_name.as_str(),
    };
    let Some(last) = latest.get(lookup_key) else {
        return true;
    };
    if last.status != new_status(result) {
        return true;
    }
    now.signed_duration_since(last.checked_at) >= REFRESH_INTERVAL
}

// _save_health_check_results_if_changed: every picked write is built with its own clock reading, then all are awaited together
pub async fn save_changed<Store>(
    rows: &Store,
    results: &[AggregatedHealthResult],
    latest: &LatestChecks,
    request: &SaveRequest<'_>,
    clock: &(dyn UtcClock + Send + Sync),
) -> bool
where
    Store: HealthCheckStore,
{
    let mut pending = Vec::new();
    for result in results {
        if !should_persist(result, latest, clock.now()) {
            continue;
        }
        let response_time_ms = elapsed_ms(clock.now(), request.start_time);
        pending.push(NewHealthCheck {
            model_name: result.model_name.clone(),
            model_id: result.model_id.clone(),
            status: new_status(result).to_owned(),
            healthy_count: result.healthy_count,
            unhealthy_count: result.unhealthy_count,
            error_message: result.error_message.clone(),
            response_time_ms: Some(response_time_ms),
            checked_by: request.checked_by.map(str::to_owned),
        });
    }
    let writes = pending.iter().map(|new| rows.persist_result(new));
    let saved = join_all(writes).await;
    saved.iter().all(Option::is_some)
}

// _save_background_health_checks_to_db: a cycle's result, written only where the stored status changed or is an hour old
pub async fn save_background<Store>(
    rows: Option<&Store>,
    cycle: HealthCycle<'_>,
    request: &SaveRequest<'_>,
    clock: &(dyn UtcClock + Send + Sync),
) -> bool
where
    Store: HealthCheckStore,
{
    let Some(store) = rows else {
        return false;
    };
    let index = model_param_index(cycle.deployments);
    let results = aggregate_results(&index, cycle.healthy, cycle.unhealthy);
    let stored = match store.stored_checks().await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::warn!("Failed to save background health checks to database: {error}");
            return false;
        }
    };
    let latest = latest_by_key(stored);
    save_changed(store, &results, &latest, request, clock).await
}

// _save_health_check_to_db: the /health route's one row for one model, whether or not the write succeeds
pub async fn save_single<Store>(
    rows: &Store,
    check: &SingleHealthCheck<'_>,
    clock: &(dyn UtcClock + Send + Sync),
) where
    Store: HealthCheckStore,
{
    let new = NewHealthCheck {
        model_name: check.model_name.to_owned(),
        model_id: check.model_id.map(str::to_owned),
        status: endpoint_status(check.healthy).to_owned(),
        healthy_count: i32::try_from(check.healthy.len()).unwrap_or(i32::MAX),
        unhealthy_count: i32::try_from(check.unhealthy.len()).unwrap_or(i32::MAX),
        error_message: first_error(check.unhealthy),
        response_time_ms: Some(elapsed_ms(clock.now(), check.start_time)),
        checked_by: check.user_id.map(str::to_owned),
    };
    drop(rows.persist_result(&new).await);
}

mod aggregate;

pub mod writes;

mod tests;
