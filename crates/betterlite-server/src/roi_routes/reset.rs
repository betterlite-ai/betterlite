use axum::Json;
use axum::extract::{Extension, State};
use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};
use betterlite_roi::settings::ROISettingsResponse;

use crate::auth::auth_identity::AuthContext;
use crate::roi_connections::access::{WRITE_FORBIDDEN, WRITE_ROLES, ensure_admin};
use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::public::public_settings;
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::{SettingsCipher, load_settings};

use super::error::{RESTART_BLOCKED, RoiRoutesError};
use super::ports::SyncStorePort;
use super::state::RoiRouteState;
use super::stored::{load_ciphertexts, save_settings};

const RESTART_STAGE: &str = "Restarting setup";
const IDLE_STAGE: &str = "Idle";

pub(super) async fn reset_setup(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISettingsResponse>, RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    let Some(repository) = state.repository.clone() else {
        return Err(RoiRoutesError::DbNotConnected);
    };
    let store = state.sync_store.store().await?;
    let owner = (state.next_id)().to_string();
    let restarting = ROISyncStatus {
        running: true,
        phase: ROISyncPhase::Spend,
        stage: RESTART_STAGE.to_owned(),
        done: 0,
        total: 0,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: None,
        finished_at: None,
        next_update: None,
        elapsed_seconds: 0,
        remaining_seconds: None,
    };
    if !store.acquire_lease(&owner, &restarting, 0.0_f64).await? {
        return Err(RoiRoutesError::Conflict(RESTART_BLOCKED));
    }
    let body = reset_settings(
        repository.as_ref(),
        state.cipher.as_ref(),
        store.as_ref(),
        state.catalog.as_deref(),
    )
    .await;
    let finished = ROISyncStatus {
        running: false,
        phase: ROISyncPhase::Idle,
        stage: IDLE_STAGE.to_owned(),
        ..restarting
    };
    match store.finish_claim(&owner, &finished, None).await {
        Err(error) => Err(error),
        Ok(_finished) => body,
    }
}

async fn reset_settings(
    repository: &dyn SettingsRepository,
    cipher: &dyn SettingsCipher,
    store: &dyn SyncStorePort,
    catalog: Option<&dyn RouterCatalog>,
) -> Result<ROISettingsResponse, RoiRoutesError> {
    let mut settings = load_settings(repository, cipher).await?;
    let stored = load_ciphertexts(repository).await?;
    settings.repos.clear();
    save_settings(repository, &settings, &stored).await?;
    store.clear_stored_report().await?;
    Ok(public_settings(&settings, catalog))
}
