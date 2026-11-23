use axum::Json;
use axum::extract::{Extension, State};
use betterlite_db::reload_schedule::status_formatting::reload_schedule_status;
use betterlite_models::source::{CatalogProvenance, SourceKind};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::caller::admin_read_snapshot;
use super::error::{ReloadError, store_call};

const STATUS_FAILED: &str = "Failed to get model cost map reload status";

#[derive(Serialize)]
pub(super) struct StatusBody {
    scheduled: bool,
    interval_hours: Option<u32>,
    last_run: Option<String>,
    next_run: Option<String>,
    source_revision: Option<String>,
    etag: Option<String>,
}

#[derive(Serialize)]
pub(super) struct SourceBody {
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
) -> Result<Json<StatusBody>, ReloadError> {
    let snapshot = admin_read_snapshot(&state, caller)?;
    let provenance = &snapshot.provenance;
    let schedule = match state.config.as_deref() {
        None => None,
        Some(store) => store_call(store.read_schedule(), STATUS_FAILED).await?,
    };
    let status = reload_schedule_status(schedule.as_ref());
    Ok(Json(StatusBody {
        scheduled: status.scheduled,
        interval_hours: status.interval_hours,
        last_run: status.last_run,
        next_run: status.next_run,
        source_revision: provenance.source_revision.clone(),
        etag: provenance.etag.clone(),
    }))
}

pub(super) async fn model_cost_map_source(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<SourceBody>, ReloadError> {
    let snapshot = admin_read_snapshot(&state, caller)?;
    let provenance = &snapshot.provenance;
    Ok(Json(source_body(provenance, snapshot.catalog.adopted_size)))
}

fn source_body(provenance: &CatalogProvenance, model_count: usize) -> SourceBody {
    let source = match provenance.kind {
        SourceKind::Local => "local",
        SourceKind::Remote => "remote",
    };
    SourceBody {
        source,
        url: provenance.url.clone(),
        is_env_forced: provenance.is_env_forced,
        fallback_reason: provenance.fallback_reason.clone(),
        loaded_at: provenance.loaded_at.map(|at| at.to_rfc3339()),
        source_revision: provenance.source_revision.clone(),
        etag: provenance.etag.clone(),
        model_count,
    }
}
