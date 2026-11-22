use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::Error as _;
use thiserror::Error;
use tracing::{info, warn};

use crate::startup_checks::error::StartupCheckError;
use crate::startup_checks::tuning::{
    Deployment, TuningBaselines, mutable_tuned_identities, snapshot_tuning_baselines,
    tuning_limit_violation,
};

pub const TUNING_BASELINE_PARAM_NAME: &str = "auto_router_tuning_baseline_v3";
pub const AUTO_ROUTER_LICENSE_REMEDY: &str =
    "A LiteLLM license with the 'auto_router' feature lifts the limit.";

// a unique violation means a competing pod recorded the first baseline between the read and the write
#[derive(Debug, Error)]
pub enum BaselineWriteError {
    #[error("another pod recorded the heuristic-v1 tuning baseline first")]
    UniqueViolation,
    #[error(transparent)]
    Database(#[from] PortError),
}

#[async_trait]
pub trait TuningBaselineStore: Send + Sync {
    async fn find_baseline(&self, param_name: &str) -> Result<Option<Json>, PortError>;
    async fn record_baseline(
        &self,
        param_name: &str,
        param_value: &str,
    ) -> Result<(), BaselineWriteError>;
}

#[derive(Debug, Error)]
enum BaselineLoadError {
    #[error(transparent)]
    Database(#[from] PortError),
    #[error("the recorded heuristic-v1 tuning baseline row cannot be decoded: {0}")]
    Decode(#[source] serde_json::Error),
}

pub async fn load_tuning_baselines(
    store: &dyn TuningBaselineStore,
    deployments: &[Deployment],
) -> Option<TuningBaselines> {
    match read_baselines(store, deployments).await {
        Ok(baselines) => Some(baselines),
        Err(error) => {
            warn!("Heuristic-v1 tuning baseline unavailable, gate not enforced this boot: {error}");
            None
        }
    }
}

async fn read_baselines(
    store: &dyn TuningBaselineStore,
    deployments: &[Deployment],
) -> Result<TuningBaselines, BaselineLoadError> {
    if let Some(row) = store.find_baseline(TUNING_BASELINE_PARAM_NAME).await? {
        return decode_tuning_baselines(&row).map_err(BaselineLoadError::Decode);
    }
    let snapshot = snapshot_tuning_baselines(deployments);
    let stored = stored_snapshot(&snapshot);
    match store
        .record_baseline(TUNING_BASELINE_PARAM_NAME, &stored)
        .await
    {
        Ok(()) => {
            let count = snapshot.len();
            info!("Recorded heuristic-v1 tuning baseline for {count} auto-router(s)");
            Ok(snapshot)
        }
        Err(BaselineWriteError::UniqueViolation) => store
            .find_baseline(TUNING_BASELINE_PARAM_NAME)
            .await?
            .map_or_else(
                || Ok(TuningBaselines::new()),
                |row| decode_tuning_baselines(&row).map_err(BaselineLoadError::Decode),
            ),
        Err(BaselineWriteError::Database(error)) => Err(BaselineLoadError::Database(error)),
    }
}

// json.dumps(dict(snapshot)): the same text the first boot writes to the litellm_config row
fn stored_snapshot(snapshot: &TuningBaselines) -> String {
    let payload: IndexMap<String, Json> = snapshot
        .iter()
        .map(|(identity, fingerprint)| (identity.clone(), Json::Text(fingerprint.clone())))
        .collect();
    dumps_object(&payload)
}

// json.loads of a text row, or the object a JSON column read returns, mapped through str()
pub fn decode_tuning_baselines(stored: &Json) -> Result<TuningBaselines, serde_json::Error> {
    let decoded = match stored {
        Json::Text(text) => serde_json::from_str::<Json>(text)?,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            stored.clone()
        }
    };
    let Json::Object(entries) = decoded else {
        return Ok(TuningBaselines::new());
    };
    let mut baselines = TuningBaselines::new();
    for (identity, fingerprint) in entries {
        let Json::Text(recorded) = fingerprint else {
            return Err(serde_json::Error::custom(format!(
                "the row maps {identity} to a value that is not a string"
            )));
        };
        baselines.insert(identity, recorded);
    }
    Ok(baselines)
}

pub async fn enforce_tuning_baseline(
    store: &dyn TuningBaselineStore,
    config_deployments: &[Deployment],
    database_deployments: Option<&[Deployment]>,
    limit: Option<usize>,
) -> Result<Option<TuningBaselines>, StartupCheckError> {
    let Some(stored_deployments) = database_deployments else {
        warn!("Heuristic-v1 tuning baseline unavailable, gate not enforced this boot");
        return Ok(None);
    };
    let mut deployments = config_deployments.to_vec();
    deployments.extend_from_slice(stored_deployments);
    let Some(baselines) = load_tuning_baselines(store, &deployments).await else {
        return Ok(None);
    };
    let mutable = mutable_tuned_identities(&deployments, &baselines);
    if let Some(violation) = tuning_limit_violation(mutable.len(), limit) {
        return Err(StartupCheckError::TuningLimit {
            violation,
            remedy: AUTO_ROUTER_LICENSE_REMEDY,
        });
    }
    Ok(Some(baselines))
}
