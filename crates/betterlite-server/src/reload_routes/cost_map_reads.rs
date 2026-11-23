use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use betterlite_db::reload_schedule::status_formatting::reload_schedule_status;
use betterlite_models::source::SourceKind;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::caller::{caller_principal, require_admin_view};
use super::error::{ReloadError, WrappedCause};

const STATUS_FAILED: &str = "Failed to get model cost map reload status";

#[derive(Serialize)]
struct StatusBody {
    scheduled: bool,
    interval_hours: Option<u32>,
    last_run: Option<String>,
    next_run: Option<String>,
    source_revision: Option<String>,
    etag: Option<String>,
}

#[derive(Serialize)]
struct SourceBody {
    source: &'static str,
    url: Option<String>,
    is_env_forced: bool,
    fallback_reason: Option<String>,
    loaded_at: Option<String>,
    source_revision: Option<String>,
    etag: Option<String>,
    model_count: usize,
}

pub(super) async fn cost_map_reload_status(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_admin_view(&principal)?;
    let snapshot = state.catalog.current();
    let provenance = &snapshot.provenance;
    let mut schedule = None;
    if let Some(store) = state.config.as_deref() {
        schedule = store
            .read_schedule()
            .await
            .map_err(|source| ReloadError::Wrapped {
                prefix: STATUS_FAILED,
                cause: WrappedCause::Store(source),
            })?;
    }
    let status = reload_schedule_status(schedule.as_ref());
    let body = StatusBody {
        scheduled: status.scheduled,
        interval_hours: status.interval_hours,
        last_run: status.last_run,
        next_run: status.next_run,
        source_revision: provenance.source_revision.clone(),
        etag: provenance.etag.clone(),
    };
    Ok(Json(body).into_response())
}

pub(super) async fn model_cost_map_source(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_admin_view(&principal)?;
    let snapshot = state.catalog.current();
    let provenance = &snapshot.provenance;
    let source = match provenance.kind {
        SourceKind::Local => "local",
        SourceKind::Remote => "remote",
    };
    let body = SourceBody {
        source,
        url: provenance.url.clone(),
        is_env_forced: provenance.is_env_forced,
        fallback_reason: provenance.fallback_reason.clone(),
        loaded_at: provenance.loaded_at.map(|at| at.to_rfc3339()),
        source_revision: provenance.source_revision.clone(),
        etag: provenance.etag.clone(),
        model_count: snapshot.catalog.adopted_size,
    };
    Ok(Json(body).into_response())
}
