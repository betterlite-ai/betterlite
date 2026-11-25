pub mod convert;
pub mod error;
pub mod health;
pub mod health_response;
pub mod history;
pub mod latest;
pub mod prisma;
pub mod query;
pub mod save;
pub mod state;
pub mod target;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use axum::Router;
use axum::routing::get;

use crate::health_status_routes::state::HealthStatusState;

// The three health routes B8540 mounts once; each sits behind the proxy's key auth middleware
pub fn health_status_routes(state: HealthStatusState) -> Router {
    Router::new()
        .route("/health", get(health::health_endpoint))
        .route(
            "/health/history",
            get(history::health_check_history_endpoint),
        )
        .route("/health/latest", get(latest::latest_health_checks_endpoint))
        .with_state(state)
}

mod tests;
