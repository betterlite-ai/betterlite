pub mod error;
pub mod query;
pub mod rows;
pub mod state;
pub mod store;

mod handlers;
mod per_user;
mod responses;
mod summary;
mod tests;
mod window;

#[path = "user_agent_analytics/tests/monthly_active_users.rs"]
mod monthly_active_users;
#[path = "user_agent_analytics/tests/per_user_analytics.rs"]
mod per_user_analytics;
#[path = "user_agent_analytics/tests/summary_tag_totals.rs"]
mod summary_tag_totals;

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
        .route(
            "/tag/mau",
            get(respond).layer(Extension(AnalyticsReport::Monthly)),
        )
        .route(
            "/tag/summary",
            get(respond).layer(Extension(AnalyticsReport::Summary)),
        )
        .route(
            "/tag/user-agent/per-user-analytics",
            get(respond).layer(Extension(AnalyticsReport::PerUser)),
        )
        .with_state(state)
}
