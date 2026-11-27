use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_server::auto_router_read_api::error::ReadApiError;
use betterlite_server::auto_router_read_api::store::AutoRouterReadStore;
use betterlite_server::benchmark_aggregates::decode::decode_session_agg_rows;
use betterlite_server::benchmark_aggregates::row::SessionAggRow;
use betterlite_server::benchmark_aggregates::window::BenchmarkWindow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SessionSeed;
use super::invalid;
use crate::router_probes::error::OperationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct QueryBounds {
    start: String,
    end: String,
    api_key: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SessionLookup {
    api_key: String,
    session_id: String,
    matched: bool,
}

// the store the reads open, fed the payload's rows and recording what each read asked for
pub(super) struct ProbeStore {
    rows: Vec<SessionAggRow>,
    sessions: Vec<(String, AutoRouterSession)>,
    query_bounds: Mutex<Option<QueryBounds>>,
    lookup: Mutex<Option<SessionLookup>>,
}

impl ProbeStore {
    pub(super) fn new(rows: &[Value], seeds: &[SessionSeed]) -> Result<Self, OperationError> {
        let mut sessions = Vec::with_capacity(seeds.len());
        for seed in seeds {
            let seed_bytes = serde_json::to_vec(&seed.row).map_err(|error| invalid(&error))?;
            let row: AutoRouterSession =
                serde_json::from_slice(&seed_bytes).map_err(|error| invalid(&error))?;
            sessions.push((seed.last_turn_at.clone(), row));
        }
        let row_bytes = serde_json::to_vec(rows).map_err(|error| invalid(&error))?;
        Ok(Self {
            rows: decode_session_agg_rows(&row_bytes).map_err(|error| invalid(&error))?,
            sessions,
            query_bounds: Mutex::new(None),
            lookup: Mutex::new(None),
        })
    }

    pub(super) fn take_query_bounds(&self) -> Option<QueryBounds> {
        self.query_bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    pub(super) fn take_lookup(&self) -> Option<SessionLookup> {
        self.lookup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn record_bounds(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) {
        *self
            .query_bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(QueryBounds {
            start: window.query_start.clone(),
            end: window.query_end.clone(),
            api_key: api_key.map(str::to_owned),
            user_id: user_id.map(str::to_owned),
        });
    }
}

#[async_trait]
impl AutoRouterReadStore for ProbeStore {
    async fn agg_rows(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        self.record_bounds(window, api_key, user_id);
        Ok(self.rows.clone())
    }

    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        let newest = self
            .sessions
            .iter()
            .filter(|(_, row)| row.hashed_api_key == api_key && row.session_id == session_id)
            .max_by(|(left, _), (right, _)| left.cmp(right))
            .map(|(_, row)| row.clone());
        *self.lookup.lock().unwrap_or_else(PoisonError::into_inner) = Some(SessionLookup {
            api_key: api_key.to_owned(),
            session_id: session_id.to_owned(),
            matched: newest.is_some(),
        });
        Ok(newest)
    }
}
