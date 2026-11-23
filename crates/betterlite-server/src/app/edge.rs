pub mod layers;
pub mod lens;
pub mod openapi;
pub mod route_directory;
pub mod ui;

use std::future::{Ready, ready};
use std::sync::Arc;

use axum::Router;

use crate::state::ServerState;

#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn mount(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its keyed routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn mount_public(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its public routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
pub fn start(_state: &Arc<ServerState>) -> Ready<()> {
    // the area owner fills this startup step
    ready(())
}

#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn wrap(router: Router, _state: &Arc<ServerState>) -> Router {
    // the edge area installs the layers that sit outside every route
    router.merge(Router::new())
}
