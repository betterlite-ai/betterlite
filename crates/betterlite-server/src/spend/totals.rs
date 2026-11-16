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

use self::prometheus::DailySpendSource;
use self::state::TotalsState;

pub fn routes<S>(state: TotalsState<S>) -> Router
where
    S: DailySpendSource + 'static,
{
    Router::new()
        .route("/global/spend/logs", get(handlers::global_spend_logs::<S>))
        .route("/global/spend", get(handlers::global_spend::<S>))
        .route("/global/spend/keys", get(handlers::global_spend_keys::<S>))
        .route(
            "/global/spend/models",
            get(handlers::global_spend_models::<S>),
        )
        .route(
            "/global/spend/teams",
            get(handlers::global_spend_teams::<S>),
        )
        .with_state(state)
}
