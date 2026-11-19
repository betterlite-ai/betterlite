pub mod error;
pub mod identity;
pub mod ports;
pub mod report;
pub mod reset;
pub mod schedule;
pub mod state;
pub mod store;
pub mod sync;

mod decode;
mod manager;
mod reports;
mod spend;
mod stored;

use axum::Router;
use axum::routing::{get, post, put};

use self::identity::update_identity_map;
use self::report::read_report;
use self::reset::reset_setup;
use self::state::RoiRouteState;
use self::sync::{cancel_sync, read_sync, start_sync};

pub const SYNC_PATH: &str = "/roi-calculator/sync";
pub const REPORT_PATH: &str = "/roi-calculator/report";
pub const IDENTITY_MAP_PATH: &str = "/roi-calculator/identity-map";
pub const SETUP_RESET_PATH: &str = "/roi-calculator/setup/reset";

pub fn routes(state: RoiRouteState) -> Router {
    Router::new()
        .route(
            SYNC_PATH,
            get(read_sync).post(start_sync).delete(cancel_sync),
        )
        .route(REPORT_PATH, get(read_report))
        .route(IDENTITY_MAP_PATH, put(update_identity_map))
        .route(SETUP_RESET_PATH, post(reset_setup))
        .with_state(state)
}

mod tests;
