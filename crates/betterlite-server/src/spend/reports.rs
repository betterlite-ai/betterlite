pub mod query;
pub mod query_service;
pub mod rows;
pub mod scope;
pub mod store;

mod caller;
mod dates;
mod error;
mod handlers;
mod sql;
mod tests;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;

use self::handlers::ReportRoute;
use self::store::ScopedReportHandle;

// B8662 merges these four paths into B069's spend routes; the module keeps its own Router for the handler tests
pub fn routes(store: ScopedReportHandle) -> Router {
    Router::new()
        .route(
            "/key/spend/report",
            get(handlers::report_respond).layer(Extension(ReportRoute::Key)),
        )
        .route(
            "/user/spend/report",
            get(handlers::report_respond).layer(Extension(ReportRoute::User)),
        )
        .route(
            "/team/spend/report",
            get(handlers::report_respond).layer(Extension(ReportRoute::Team)),
        )
        .route(
            "/organization/spend/report",
            get(handlers::report_respond).layer(Extension(ReportRoute::Organization)),
        )
        .with_state(store)
}
