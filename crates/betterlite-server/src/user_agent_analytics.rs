pub mod error;
pub mod query;
pub mod rows;
pub mod state;
pub mod store;

mod handlers;
mod responses;
mod tests;
mod window;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;

use self::handlers::{AnalyticsReport, respond};
use self::state::UserAgentAnalyticsState;

pub fn routes(state: UserAgentAnalyticsState) -> Router {
    Router::new()
        .route(
            "/tag/distinct",
            get(respond).layer(Extension(AnalyticsReport::Distinct)),
        )
        .route(
            "/tag/dau",
            get(respond).layer(Extension(AnalyticsReport::Daily)),
        )
        .route(
            "/tag/wau",
            get(respond).layer(Extension(AnalyticsReport::Weekly)),
        )
        .with_state(state)
}
