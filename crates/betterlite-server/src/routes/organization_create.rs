use std::sync::Arc;

use axum::Router;
use betterlite_auth::auth_model_patterns::view::{Deployment, DeploymentView, ModelAccessView};
use indexmap::IndexMap;

use crate::app::spend::cli_writer_store;
use crate::organization_create::creation::OrganizationCreateState;
use crate::organization_create::http;
use crate::state::ServerState;

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    http::mount(Arc::new(OrganizationCreateState {
        database: cli_writer_store(state),
        router_configured: !state.executor.deployments.is_empty(),
        view: router_view(state),
        max_recurse_depth: state.auth_model_patterns.max_recurse_depth,
    }))
}

// the routing facts can_user_call_model reads: the deployments behind each model group
#[cfg_attr(test, mutants::skip)]
fn router_view(state: &Arc<ServerState>) -> Arc<dyn ModelAccessView> {
    let deployments = state
        .executor
        .deployments
        .iter()
        .map(|deployment| Deployment {
            model_name: deployment.model_group.clone(),
            access_groups: Vec::new(),
            team_id: None,
            team_public_model_name: None,
        })
        .collect();
    Arc::new(DeploymentView {
        deployments,
        model_group_alias: IndexMap::new(),
    })
}
