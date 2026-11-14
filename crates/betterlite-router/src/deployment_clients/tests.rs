#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_clients::guard::MaxParallelRequestsLimit;
use crate::deployment_identity::spec::DeploymentSpec;

mod deployment_clients_behaviors;
mod deployment_clients_boundaries;

const CLIENT_TTL_SECONDS: u64 = 3600;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn spec(
    model_name: &str,
    id: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm_params = JsonObject::from_iter([("model".to_owned(), text("openai/gpt-4o"))]);
    for (key, value) in params {
        litellm_params.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::from_iter([("id".to_owned(), text(id))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .expect("deployment")
}

fn catalog(specs: &[DeploymentSpec]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for built in specs {
        catalog.model_names.insert(built.model_name.clone());
        catalog.index_patterns(built).expect("pattern");
        catalog.push(built.clone());
    }
    catalog
}

fn limit(max_parallel_requests: i64) -> Arc<MaxParallelRequestsLimit> {
    Arc::new(MaxParallelRequestsLimit::new(
        max_parallel_requests,
        String::from("d1"),
        String::from("gpt"),
    ))
}

fn clock_at(millis: u64) -> Arc<Mutex<u64>> {
    Arc::new(Mutex::new(millis))
}

fn system_clock(millis_cell: &Arc<Mutex<u64>>) -> Arc<dyn Fn() -> SystemTime + Send + Sync> {
    let cell = Arc::clone(millis_cell);
    Arc::new(move || {
        let millis = *cell.lock().expect("clock");
        UNIX_EPOCH
            .checked_add(Duration::from_millis(millis))
            .unwrap_or(UNIX_EPOCH)
    })
}
