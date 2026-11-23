pub mod layers;
pub mod lens;
pub mod openapi;
pub mod route_directory;
pub mod ui;

mod tests;

use std::future::{Ready, ready};
use std::sync::Arc;

use axum::Router;
use betterlite_config::loader::startup::process_environment;
use betterlite_openapi::docs_settings::DocsEnv;

use crate::docs_routes::{DocsRouteConfig, docs_router};
use crate::public_endpoints::providers::providers_routes;
use crate::state::ServerState;

#[cfg_attr(test, mutants::skip)]
pub fn mount(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its keyed routes
    router.merge(Router::new())
}

pub fn mount_public(router: Router, state: &Arc<ServerState>) -> Router {
    let environment = process_environment();
    let urls = DocsEnv {
        docs_url: environment.get("DOCS_URL").cloned(),
        redoc_url: environment.get("REDOC_URL").cloned(),
        openapi_url: environment.get("OPENAPI_URL").cloned(),
        no_docs: environment.get("NO_DOCS").cloned(),
        no_redoc: environment.get("NO_REDOC").cloned(),
        no_openapi: environment.get("NO_OPENAPI").cloned(),
        root_redirect_url: environment.get("ROOT_REDIRECT_URL").cloned(),
    }
    .resolve();
    let server_root_path = state
        .config
        .sections
        .request_root_paths
        .server_root_path
        .clone();
    let title = state.config.sections.public_discovery.docs_title.clone();
    let docs = docs_router(DocsRouteConfig {
        urls: urls.clone(),
        title: title.clone(),
        server_root_path: server_root_path.clone(),
    });
    let document = openapi::document_router(urls.openapi_url.as_deref(), &server_root_path, &title);
    router.merge(providers_routes()).merge(docs).merge(document)
}

#[cfg_attr(test, mutants::skip)]
pub fn start(_state: &Arc<ServerState>) -> Ready<()> {
    // the area owner fills this startup step
    ready(())
}

#[cfg_attr(test, mutants::skip)]
pub fn wrap(router: Router, _state: &Arc<ServerState>) -> Router {
    // the edge area installs the layers that sit outside every route
    router.merge(Router::new())
}
