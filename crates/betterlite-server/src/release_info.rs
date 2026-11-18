pub mod bullets;
pub mod records;
mod route;
mod service;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use betterlite_context::dynamic_logging_cache::Clock;
use secrecy::{ExposeSecret as _, SecretString};

use self::records::FIXED_RELEASE_URL;
use self::service::ReleaseService;
use crate::state::ServerState;

pub const RELEASE_FIXTURE_VARIABLE: &str = "BETTERLITE_LATEST_RELEASE_URL";

#[must_use]
pub fn routes(state: &Arc<ServerState>) -> Router {
    let clock: Clock = Arc::new(system_seconds);
    let source = release_source_for(&state.config.environment_variables, cfg!(debug_assertions));
    let service = Arc::new(ReleaseService::new(
        state.executor.client.clone(),
        source,
        clock,
    ));
    route::router(service)
}

type Environment = BTreeMap<String, Option<SecretString>>;

fn release_source_for(environment: &Environment, fixture_enabled: bool) -> String {
    let configured = if fixture_enabled {
        environment.get(RELEASE_FIXTURE_VARIABLE)
    } else {
        None
    };
    match configured.and_then(Option::as_ref) {
        Some(source) => source.expose_secret().to_owned(),
        None => FIXED_RELEASE_URL.to_owned(),
    }
}

fn system_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
}
