use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use betterlite_server::auto_router_read_api::VIEW_ACTION;
use betterlite_server::auto_router_read_api::query::{BenchmarksQuery, SessionQuery};
use betterlite_server::auto_router_read_api::service::{benchmarks_for, session_for};
use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::routing::auto_router_read_api::ConfiguredRouter;
use betterlite_server::shadow_eval_lifecycle::access::require_admin;
use betterlite_server::shadow_eval_validation::access::AdminRole;

use super::store::{ProbeStore, QueryBounds, SessionLookup};
use super::{BenchmarksStep, Payload, SessionStep, invalid};
use crate::router_probes::error::OperationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum StepOutcome {
    Benchmarks {
        response: Value,
        query: QueryBounds,
    },
    Session {
        response: Value,
        lookup: SessionLookup,
    },
    Failed {
        status: u16,
    },
}

fn identity(role: Option<&str>) -> Identity {
    Identity {
        user_id: Some("probe".to_owned()),
        user_role: role.map(str::to_owned),
    }
}

fn deployments(payload: &Payload) -> Vec<ConfiguredRouter> {
    payload
        .deployments
        .iter()
        .map(|deployment| ConfiguredRouter {
            model_name: deployment.model_name.clone().unwrap_or_default(),
            model: deployment.model.clone(),
        })
        .collect()
}

// FastAPI checks the role first, then the database, then the dates and the rollup read
pub(super) async fn benchmarks_step(
    payload: &Payload,
    step: &BenchmarksStep,
    today: NaiveDate,
) -> Result<StepOutcome, OperationError> {
    if let Err(error) = require_admin(
        &identity(step.role.as_deref()),
        VIEW_ACTION,
        AdminRole::Viewer,
    ) {
        return Ok(StepOutcome::Failed {
            status: error.status_code(),
        });
    }
    if !step.database {
        return Ok(StepOutcome::Failed { status: 500 });
    }
    let store = ProbeStore::from_rows(&step.rows)?;
    let query = BenchmarksQuery {
        start_date: step.start_date.clone(),
        end_date: step.end_date.clone(),
        api_key: step.api_key.clone(),
        user_id: step.user_id.clone(),
    };
    match benchmarks_for(&store, &query, &deployments(payload), today).await {
        Ok(response) => Ok(StepOutcome::Benchmarks {
            response: serde_json::to_value(response).map_err(|error| invalid(&error))?,
            query: store
                .take_query_bounds()
                .ok_or_else(|| invalid("the benchmarks read never reached the store"))?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}

// the session read needs no role: the key in the input is the caller's partition
pub(super) async fn session_step(step: &SessionStep) -> Result<StepOutcome, OperationError> {
    let store = ProbeStore::from_sessions(&step.sessions)?;
    let query = SessionQuery {
        session_id: step.session_id.clone(),
    };
    match session_for(&store, Some(step.api_key.as_str()), &query).await {
        Ok(response) => Ok(StepOutcome::Session {
            response: serde_json::to_value(response).map_err(|error| invalid(&error))?,
            lookup: store
                .take_lookup()
                .ok_or_else(|| invalid("the session read never reached the store"))?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}
