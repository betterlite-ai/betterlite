mod group_patch;
mod tests;

use crate::scim_directory_reads;
use crate::scim_discovery;
use crate::scim_group_lifecycle;
use crate::scim_placeholder_repair;
use crate::scim_user_lifecycle;
use crate::scim_user_patch;
use crate::state::ServerState;
use axum::Router;
use std::sync::Arc;

// every stub router is empty, so a replacement mutation of this merge equals its body
#[cfg_attr(test, mutants::skip)]
pub(crate) fn scim_router(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(scim_user_lifecycle::routes(state))
        .merge(scim_user_patch::routes(state))
        .merge(scim_group_lifecycle::routes(state))
        .merge(scim_directory_reads::routes(state))
        .merge(scim_discovery::routes(state))
        .merge(scim_placeholder_repair::routes(state))
        .merge(group_patch::routes(state))
}
