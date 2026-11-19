use axum::Json;
use axum::extract::{Extension, State};
use betterlite_roi::settings::ROISettingsResponse;

use crate::auth::auth_identity::AuthContext;
use crate::roi_github::repositories::test_repositories;
use crate::roi_github::transport::Transport;
use crate::roi_settings::public::public_settings;
use crate::roi_settings::store::load_settings;

use super::super::access::{WRITE_FORBIDDEN, WRITE_ROLES, ensure_admin};
use super::super::error::RoiConnectionsError;
use super::super::gateway_http_client::GatewayHttpClient;
use super::super::state::{RoiConnectionsState, github_client, require_repository};
use super::super::test_estimator_access::test_estimator_access;

pub const CONNECTIONS_TEST_PATH: &str = "/roi-calculator/connections/test";
pub const NOT_READY: &str = "Choose repositories and an available estimator model first.";

pub(super) async fn test_connections<T>(
    State(state): State<RoiConnectionsState<T>>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISettingsResponse>, RoiConnectionsError>
where
    T: Transport + Clone + Send + Sync + 'static,
{
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    let repository = require_repository(&state)?;
    let settings = load_settings(repository, state.cipher.as_ref()).await?;
    let public = public_settings(&settings, state.catalog.as_deref());
    if !public.ready {
        return Err(RoiConnectionsError::Conflict(NOT_READY));
    }
    let client = GatewayHttpClient {
        base: state.gateway_base.clone(),
        transport: state.http.clone(),
    };
    test_estimator_access(&settings, state.master_key.as_ref(), &client).await?;
    let github = github_client(&settings, &state)?;
    let check = test_repositories(&github, &settings.repos).await;
    github.close();
    check?;
    Ok(Json(public))
}

#[path = "connections/tests/auth.rs"]
mod auth;
#[path = "connections/tests/errors.rs"]
mod errors;
#[path = "connections/tests/success.rs"]
mod success;
