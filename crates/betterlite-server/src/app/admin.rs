pub mod config;
pub mod debugging;
pub mod models;
pub mod prompts;
pub mod public_hubs;
pub mod roi;
pub mod search_usage;
pub mod urls;
pub mod usage_chat;

use std::future::{Ready, ready};
use std::sync::Arc;

use axum::Router;

use crate::state::ServerState;

#[cfg_attr(test, mutants::skip)]
pub fn mount(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its keyed routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
pub fn mount_public(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its public routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
pub fn start(_state: &Arc<ServerState>) -> Ready<()> {
    // the area owner fills this startup step
    ready(())
}
