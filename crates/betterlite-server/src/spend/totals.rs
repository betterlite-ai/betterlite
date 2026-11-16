mod access;
mod handlers;
mod internal;
pub mod prometheus;
pub mod query;
mod responses;
pub mod rows;
pub mod state;
pub mod store;
mod teams;
mod tests;

use axum::Router;
use axum::routing::get;

use self::state::TotalsState;

pub fn routes(state: TotalsState) -> Router {
    Router::new()
        .route("/global/spend/logs", get(handlers::global_spend_logs))
        .route("/global/spend", get(handlers::global_spend))
        .route("/global/spend/keys", get(handlers::global_spend_keys))
        .route("/global/spend/models", get(handlers::global_spend_models))
        .route("/global/spend/teams", get(handlers::global_spend_teams))
        .with_state(state)
}
