pub mod catalog;
pub mod core;
pub mod gateway_sessions;
pub mod identity;
pub mod llm_tools;
pub mod oauth_browser;
pub mod oauth_flows;
pub mod oauth_routes;
pub mod python_tools;
pub mod rest;
pub mod server_admin;
pub mod state;
pub mod tokens;
pub mod toolsets;
pub mod transport;
pub mod upstream;
pub mod user_routes;

use std::future::{Ready, ready};
use std::sync::Arc;

use axum::Router;

use crate::routes::mcp_byok_consent;
use crate::state::ServerState;

#[cfg_attr(test, mutants::skip)]
pub fn mount(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its keyed routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
pub fn mount_public(router: Router, state: &Arc<ServerState>) -> Router {
    // the consent page is unauthenticated on the GET and checks the UI cookie itself on the POST, so it sits outside the auth layers
    router.merge(mcp_byok_consent::routes(state))
}

#[cfg_attr(test, mutants::skip)]
pub fn start(_state: &Arc<ServerState>) -> Ready<()> {
    // the area owner fills this startup step
    ready(())
}
