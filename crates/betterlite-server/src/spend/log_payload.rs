pub mod cold_storage;
pub mod content;
pub mod convert;
pub mod daily_summary;
pub mod dates;
pub mod error;
mod handler;
pub mod metadata;
pub mod ports;
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use self::cold_storage::ColdStorageHandler;
use self::ports::ActiveLoggers;
use crate::spend::visibility::service::SpendLogVisibility;

#[derive(Clone)]
pub struct LogPayloadState {
    // None when the proxy runs without a database, so only an admin may read callback storage
    pub visibility: Option<Arc<SpendLogVisibility>>,
    pub loggers: Arc<dyn ActiveLoggers>,
    pub cold_storage: Arc<ColdStorageHandler>,
}

pub fn routes(state: LogPayloadState) -> Router {
    Router::new()
        .route(
            "/spend/logs/ui/{request_id}",
            get(handler::view_request_payload),
        )
        .with_state(state)
}
