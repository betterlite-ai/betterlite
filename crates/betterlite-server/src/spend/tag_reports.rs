mod aggregation;
mod caller;
mod handlers;
pub mod query;
mod responses;
pub mod rows;
pub mod store;
mod tests;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;

use self::handlers::{TableReport, TagReport, table_respond, tag_respond};
use self::store::TagReportHandle;

pub fn routes(store: TagReportHandle) -> Router {
    Router::new()
        .route(
            "/spend/keys",
            get(table_respond).layer(Extension(TableReport::Keys)),
        )
        .route(
            "/spend/users",
            get(table_respond).layer(Extension(TableReport::Users)),
        )
        .route(
            "/spend/tags",
            get(tag_respond).layer(Extension(TagReport::Tags)),
        )
        .route(
            "/global/spend/all_tag_names",
            get(tag_respond).layer(Extension(TagReport::TagNames)),
        )
        .route(
            "/global/spend/tags",
            get(tag_respond).layer(Extension(TagReport::DailyTags)),
        )
        .with_state(store)
}
