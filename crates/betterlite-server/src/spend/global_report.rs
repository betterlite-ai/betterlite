pub mod query;
pub mod rows;
pub mod store;

mod error;
mod handler;
mod sql;
mod tests;

use axum::Router;
use axum::routing::get;

use self::store::GlobalReportHandle;

// B8662 merges this path into B069's spend routes; the module keeps its own Router for the handler tests
pub fn routes(store: GlobalReportHandle) -> Router {
    Router::new()
        .route("/global/spend/report", get(handler::global_report_respond))
        .with_state(store)
}
