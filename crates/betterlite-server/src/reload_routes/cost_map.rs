use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use betterlite_models::reload::ReloadOutcome;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::caller::{caller_principal, require_proxy_admin};
use super::error::{ReloadError, WrappedCause};

const RELOAD_FAILED: &str = "Failed to reload model cost map";

#[derive(Serialize)]
struct ReloadBody {
    message: String,
    status: &'static str,
    models_count: usize,
    timestamp: String,
    source_revision: Option<String>,
    etag: Option<String>,
}

pub(super) async fn reload_model_cost_map(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_proxy_admin(&principal)?;
    let Some(store) = state.config.as_deref() else {
        return Err(ReloadError::MissingDatabase);
    };
    let snapshot = match state.catalog.reload().await {
        ReloadOutcome::Reloaded(snapshot) => snapshot,
        ReloadOutcome::Unavailable(unavailable) => {
            return Err(ReloadError::CostMapUnavailable {
                reason: unavailable.reason,
            });
        }
    };
    // LiteLLM reads the clock after the fetch, so the timestamp is the moment the map is adopted
    let now = (state.clock)();
    let models_count = state.reloads.swap_in_model_cost_map(&snapshot, now);
    let revision =
        store
            .record_manual_reload(now)
            .await
            .map_err(|source| ReloadError::Wrapped {
                prefix: RELOAD_FAILED,
                cause: WrappedCause::Store(source),
            })?;
    state.reloads.set_cost_map_applied_revision(revision);
    tracing::info!(
        "Model cost map reloaded successfully in current pod. Models count: {models_count}"
    );
    let provenance = &snapshot.provenance;
    let body = ReloadBody {
        message: format!("Price data reloaded successfully! {models_count} models updated."),
        status: "success",
        models_count,
        timestamp: now.to_rfc3339(),
        source_revision: provenance.source_revision.clone(),
        etag: provenance.etag.clone(),
    };
    Ok(Json(body).into_response())
}
