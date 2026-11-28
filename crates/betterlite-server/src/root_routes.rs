mod error;
mod wire;

mod tests;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, State};
use axum::routing::get;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_router::adaptive_selection::StateSnapshot;

use crate::auth::auth_identity::AuthContext;
use crate::management_guards::caller::caller_has_admin_view;

use self::error::RootRouteError;
use self::wire::{RoutersBody, SnapshotWire};

pub const HOME_PATH: &str = "/";
pub const ADAPTIVE_ROUTER_STATE_PATH: &str = "/adaptive_router/state";

// LiteLLM's llm_router.adaptive_routers: the deployments under each router key, in the registry's own order; an empty outer list is both "no router" and "no adaptive routers"
pub trait AdaptiveRouterSnapshots: Send + Sync {
    fn snapshots(&self) -> Vec<Vec<StateSnapshot>>;
}

#[derive(Clone)]
pub struct RootRouteState {
    // read per request, so a router the database sync installs later answers too
    pub adaptive_routers: Arc<dyn AdaptiveRouterSnapshots>,
}

pub fn routes(state: RootRouteState) -> Router {
    Router::new()
        .route(HOME_PATH, get(|| async { Json("LiteLLM: RUNNING") }))
        .route(
            ADAPTIVE_ROUTER_STATE_PATH,
            get(
                |State(root_state): State<RootRouteState>,
                 caller: Option<Extension<AuthContext>>| async move {
                    adaptive_router_state(
                        &root_state,
                        caller.as_ref().map(|extension| &extension.0.principal),
                    )
                },
            ),
        )
        .with_state(state)
}

fn adaptive_router_state(
    state: &RootRouteState,
    principal: Option<&Principal>,
) -> Result<Json<RoutersBody>, RootRouteError> {
    if !principal.is_some_and(caller_has_admin_view) {
        return Err(RootRouteError::AdminViewRequired);
    }
    let snapshots = state.adaptive_routers.snapshots();
    if snapshots.is_empty() {
        return Err(RootRouteError::NoAdaptiveRouter);
    }
    let routers = snapshots
        .into_iter()
        .flatten()
        .map(SnapshotWire::from)
        .collect();
    Ok(Json(RoutersBody { routers }))
}
