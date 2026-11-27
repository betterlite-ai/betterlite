pub mod chat_stream;
pub mod containers;
pub mod eval_deployments;
pub mod evals;
pub mod gateway_compaction;
pub mod google;
pub mod google_agents;
pub mod google_count_tokens;
pub mod google_generation;
pub mod google_sdk_call;
pub mod limit_parity;
pub mod native_compaction;
pub mod pre_call;
pub mod pre_call_hooks;
pub mod priority_parity;
pub mod prompt_runtime;
pub mod provider_chat_step;
pub mod rag;
pub mod response_cache;
pub mod route_ports;
pub mod route_pre_call;
pub mod router_calls;
pub mod router_completion;
pub mod router_routing;
pub mod sdk_hooks;
pub mod search_routes;
pub mod shared_stream_ports;
pub mod stream_generator;
pub mod utility;
pub mod video_runtime;

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
