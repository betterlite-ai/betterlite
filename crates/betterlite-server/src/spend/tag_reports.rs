mod aggregation;
mod caller;
mod handlers;
pub mod query;
mod responses;
pub mod rows;
pub mod store;
mod tests;

use axum::Router;
use axum::routing::get;

use self::handlers::{all_tag_names, global_spend_tags, spend_keys, spend_tags, spend_users};
use self::store::TagReportHandle;

pub fn routes(store: TagReportHandle) -> Router {
    Router::new()
        .route("/spend/keys", get(spend_keys))
        .route("/spend/users", get(spend_users))
        .route("/spend/tags", get(spend_tags))
        .route("/global/spend/all_tag_names", get(all_tag_names))
        .route("/global/spend/tags", get(global_spend_tags))
        .with_state(store)
}
