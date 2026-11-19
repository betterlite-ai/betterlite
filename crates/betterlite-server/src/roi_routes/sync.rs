use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_roi::records::{ROIReport, ROISyncStatus};

use crate::auth::auth_identity::AuthContext;
use crate::roi_connections::access::{
    READ_FORBIDDEN, READ_ROLES, WRITE_FORBIDDEN, WRITE_ROLES, ensure_admin,
};
use crate::roi_settings::deployments::router_estimator_models;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::public::public_settings;
use crate::roi_settings::store::{REPORT_KEY, load_parameter, load_settings};

use super::error::{ALREADY_RUNNING, NOT_READY, RoiRoutesError};
use super::ports::RouteSyncRun;
use super::schedule::next_update;
use super::state::RoiRouteState;

pub(super) async fn read_sync(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISyncStatus>, RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, READ_ROLES, READ_FORBIDDEN)?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiRoutesError::DbNotConnected);
    };
    let store = state.sync_store.store().await?;
    let stored = store.read_status().await?;
    let status = stored.unwrap_or_else(|| state.manager.current_status());
    let settings = load_settings(repository, state.cipher.as_ref()).await?;
    let report =
        load_parameter::<ROIReport, _>(repository, REPORT_KEY, RoiSettingsError::StoredReport)
            .await?;
    let next = next_update(&settings, &status, report.as_ref())?;
    Ok(Json(ROISyncStatus {
        next_update: next.map(isoformat),
        ..status
    }))
}

pub(super) async fn start_sync(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<(StatusCode, Json<ROISyncStatus>), RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    let Some(repository) = state.repository.clone() else {
        return Err(RoiRoutesError::DbNotConnected);
    };
    let settings = load_settings(repository.as_ref(), state.cipher.as_ref()).await?;
    let public = public_settings(&settings, state.catalog.as_deref());
    if !public.ready {
        return Err(RoiRoutesError::Conflict(NOT_READY));
    }
    let complete = (state.completion_caller)(&settings)?;
    let estimator_models =
        router_estimator_models(state.catalog.as_deref(), &settings.estimator_model);
    let spend_reader = (state.spend_reader)();
    let store = state.sync_store.store().await?;
    let run = RouteSyncRun {
        settings,
        repository,
        spend_reader,
        complete,
        estimator_models,
        store: Some(store),
        scheduled_interval: 0.0_f64,
    };
    if !state.manager.start_run(run).await {
        return Err(RoiRoutesError::Conflict(ALREADY_RUNNING));
    }
    Ok((StatusCode::ACCEPTED, Json(state.manager.current_status())))
}

pub(super) async fn cancel_sync(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISyncStatus>, RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    if state.repository.is_none() {
        return Err(RoiRoutesError::DbNotConnected);
    }
    let store = state.sync_store.store().await?;
    store.cancel_sync().await?;
    state.manager.cancel_active_run().await;
    let status = store
        .read_status()
        .await?
        .unwrap_or_else(|| state.manager.current_status());
    Ok(Json(status))
}
