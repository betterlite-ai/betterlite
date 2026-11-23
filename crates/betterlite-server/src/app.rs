pub mod admin;
pub mod agents;
pub mod cache_secrets;
pub mod callbacks;
pub mod edge;
pub mod guardrails;
pub mod health;
pub mod inference;
pub mod management;
pub mod mcp;
pub mod pass_through;
pub mod spend;

mod tests;

use std::sync::Arc;

use axum::Router;

use crate::state::ServerState;

#[must_use]
pub fn mount(application: Router, state: &Arc<ServerState>) -> Router {
    let mut router = mcp::mount(application, state);
    router = inference::mount(router, state);
    router = pass_through::mount(router, state);
    router = agents::mount(router, state);
    router = cache_secrets::mount(router, state);
    router = callbacks::mount(router, state);
    router = spend::mount(router, state);
    router = guardrails::mount(router, state);
    router = management::mount(router, state);
    router = admin::mount(router, state);
    router = health::mount(router, state);
    edge::mount(router, state)
}

#[must_use]
pub fn mount_public(application: Router, state: &Arc<ServerState>) -> Router {
    let with_mcp = mcp::mount_public(application, state);
    let with_inference = inference::mount_public(with_mcp, state);
    let with_pass_through = pass_through::mount_public(with_inference, state);
    let with_agents = agents::mount_public(with_pass_through, state);
    let with_cache_secrets = cache_secrets::mount_public(with_agents, state);
    let with_callbacks = callbacks::mount_public(with_cache_secrets, state);
    let with_spend = spend::mount_public(with_callbacks, state);
    let with_guardrails = guardrails::mount_public(with_spend, state);
    let with_management = management::mount_public(with_guardrails, state);
    let with_admin = admin::mount_public(with_management, state);
    let with_health = health::mount_public(with_admin, state);
    edge::mount_public(with_health, state)
}

#[cfg_attr(test, mutants::skip)]
pub async fn start(state: &Arc<ServerState>) {
    mcp::start(state).await;
    inference::start(state).await;
    pass_through::start(state).await;
    agents::start(state).await;
    cache_secrets::start(state).await;
    callbacks::start(state).await;
    spend::start(state).await;
    guardrails::start(state).await;
    management::start(state).await;
    admin::start(state).await;
    health::start(state).await;
    edge::start(state).await;
}

#[must_use]
pub fn wrap(router: Router, state: &Arc<ServerState>) -> Router {
    // the edge area installs the layers that sit outside every route
    edge::wrap(router, state)
}
