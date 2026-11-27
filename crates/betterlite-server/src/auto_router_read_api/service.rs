use std::collections::BTreeMap;

use betterlite_db::autorouter_facts::session::bounded_session_id;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_logging::analytics::python;
use betterlite_models::capture_rate::format::{
    serialize_money, serialize_optional_money,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::benchmark_aggregates::group::{Benchmarks, benchmarks};
use crate::benchmark_aggregates::row::savings_cohort;
use crate::benchmark_aggregates::window::BenchmarkWindow;
use crate::routing::auto_router_read_api::ConfiguredRouter;
use crate::routing::benchmark_aggregates::StrategyDeployment;

use super::error::ReadApiError;
use super::query::{BenchmarksQuery, SessionQuery};
use super::store::AutoRouterReadStore;

// AutoRouterSessionResponse: the session's last route and its spend against the savings baseline
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub router_name: String,
    pub router_type: String,
    pub turns: i64,
    pub last_model: String,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    pub savings_estimated_turns: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub savings_estimated_actual_spend: Decimal,
    #[serde(serialize_with = "serialize_optional_money")]
    pub saved_spend: Option<Decimal>,
    #[serde(serialize_with = "serialize_optional_money")]
    pub baseline_spend: Option<Decimal>,
    #[serde(serialize_with = "serialize_optional_money")]
    pub savings_estimated_baseline_spend: Option<Decimal>,
    pub baseline_model: Option<String>,
    pub baseline_models: BTreeMap<String, i64>,
}

// get_auto_router_benchmarks after the role and database checks: the window's rows, compared and grouped
pub async fn benchmarks_for(
    store: &dyn AutoRouterReadStore,
    query: &BenchmarksQuery,
    deployments: &[ConfiguredRouter],
    today: NaiveDate,
) -> Result<Benchmarks, ReadApiError> {
    let window = BenchmarkWindow::resolve(
        query.start_date.as_deref(),
        query.end_date.as_deref(),
        today,
    )?;
    let rows = store
        .agg_rows(&window, query.api_key.as_deref(), query.user_id.as_deref())
        .await?;
    let configured: Vec<StrategyDeployment<'_>> =
        deployments.iter().map(ConfiguredRouter::deployment).collect();
    // an empty registry and LiteLLM's absent router both leave the idle list empty
    Ok(benchmarks(&window, &rows, Some(&configured)))
}

// get_auto_router_session: one row under the caller's own key, bounded the way the writer bounded the id
pub async fn session_for(
    store: &dyn AutoRouterReadStore,
    api_key: Option<&str>,
    query: &SessionQuery,
) -> Result<SessionResponse, ReadApiError> {
    let Some(api_key) = api_key else {
        // a caller without a key binds NULL, which matches no row: the same 404 without the round trip
        return Err(session_not_found(&query.session_id)?);
    };
    let session_id = bounded_session_id(&query.session_id);
    let Some(row) = store.latest_session(api_key, &session_id).await? else {
        return Err(session_not_found(&query.session_id)?);
    };
    Ok(session_response(&query.session_id, &row))
}

// python's {session_id!r}, the form LiteLLM prints the raw query value in
fn session_not_found(session_id: &str) -> Result<ReadApiError, ReadApiError> {
    let session = python::repr(session_id).map_err(|source| ReadApiError::Store {
        message: source.to_string(),
    })?;
    Ok(ReadApiError::SessionNotFound { session })
}

fn session_response(session_id: &str, row: &AutoRouterSession) -> SessionResponse {
    let (saved_spend, baseline_spend) = savings_cohort(
        row.turns,
        row.estimated_turns,
        row.spend,
        row.saved_spend,
    );
    let (_estimated_saved, estimated_baseline_spend) = savings_cohort(
        row.turns,
        row.estimated_turns,
        row.estimated_actual_spend,
        row.estimated_saved_spend,
    );
    SessionResponse {
        session_id: session_id.to_owned(),
        router_name: row.router_name.clone(),
        router_type: row.router_type.clone(),
        turns: row.turns,
        last_model: row.last_model.clone(),
        spend: row.spend,
        savings_estimated_turns: row.estimated_turns,
        savings_estimated_actual_spend: row.estimated_actual_spend,
        saved_spend,
        baseline_spend,
        savings_estimated_baseline_spend: estimated_baseline_spend,
        baseline_model: row.baseline_model().map(str::to_owned),
        baseline_models: row.baseline_models.clone(),
    }
}
