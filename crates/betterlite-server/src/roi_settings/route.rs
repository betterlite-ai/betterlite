use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use axum::routing::get;
use betterlite_roi::settings::ROISettingsResponse;
use betterlite_roi::settings::ROISettingsUpdate;
use serde::de::DeserializeOwned;

use crate::auth::auth_identity::AuthContext;

use super::auth::READ_FORBIDDEN;
use super::auth::READ_ROLES;
use super::auth::WRITE_FORBIDDEN;
use super::auth::WRITE_ROLES;
use super::auth::require_admin;
use super::error::InternalError;
use super::error::RoiSettingsError;
use super::error::request_issues;
use super::patch::PatchPresence;
use super::patch::build_settings;
use super::patch::patched_secrets;
use super::public::public_settings;
use super::state::RoiSettingsState;
use super::store::StoredSettings;
use super::store::load_settings;
use super::store::load_stored;
use super::store::open_settings;
use super::store::save_settings;
use super::store::validate_settings;

pub fn router(state: RoiSettingsState) -> Router {
    Router::new()
        .route(
            "/roi-calculator/settings",
            get(get_settings).put(update_settings),
        )
        .with_state(state)
}

async fn get_settings(
    State(state): State<RoiSettingsState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISettingsResponse>, RoiSettingsError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    require_admin(context, READ_ROLES, READ_FORBIDDEN)?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiSettingsError::Internal(InternalError::DbNotConnected));
    };
    let settings = load_settings(repository, state.cipher.as_ref()).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_deref())))
}

async fn update_settings(
    State(state): State<RoiSettingsState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ROISettingsResponse>, RoiSettingsError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    require_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiSettingsError::Internal(InternalError::DbNotConnected));
    };
    let (patch, presence) = parse_update_body(&body)?;
    let issues = request_issues(&patch);
    if !issues.is_empty() {
        return Err(RoiSettingsError::Request(issues));
    }
    let stored = load_stored(repository).await?;
    let current = validate_settings(open_settings(&stored, state.cipher.as_ref()))?;
    if presence.github_api_url == Some(None) {
        return Err(RoiSettingsError::NullUrl);
    }
    let secrets = patched_secrets(&patch, &presence, &stored, &current, state.cipher.as_ref())?;
    let settings = build_settings(patch, &current, &secrets)?;
    let record = StoredSettings {
        github_api_url: settings.github_api_url.clone(),
        github_token: secrets.encrypted_token,
        estimator_key: secrets.encrypted_estimator_key,
        repos: settings.repos.clone(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        identity_map: settings.identity_map.clone(),
    };
    save_settings(repository, &record).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_deref())))
}

pub fn parse_update_body(
    body: &[u8],
) -> Result<(ROISettingsUpdate, PatchPresence), RoiSettingsError> {
    let patch: ROISettingsUpdate = decode_body(body)?;
    let presence: PatchPresence = decode_body(body)?;
    Ok((patch, presence))
}

fn decode_body<T>(body: &[u8]) -> Result<T, RoiSettingsError>
where
    T: DeserializeOwned,
{
    if body.is_empty() {
        return Err(RoiSettingsError::MissingBody);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let value = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| RoiSettingsError::Body(Box::new(source)))?;
    deserializer
        .end()
        .map_err(|_error| RoiSettingsError::InvalidJson)?;
    Ok(value)
}
