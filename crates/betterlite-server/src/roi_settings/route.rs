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

use super::auth::read_admin;
use super::auth::write_admin;
use super::error::InternalError;
use super::error::RoiSettingsError;
use super::error::request_issues;
use super::patch::PatchPresence;
use super::patch::build_settings;
use super::patch::patched_secrets;
use super::patch::stored_from;
use super::public::public_settings;
use super::state::RoiSettingsState;
use super::store::load_settings;
use super::store::load_stored;
use super::store::save_settings;

#[must_use]
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
    read_admin(caller.as_ref().map(|extension| &extension.0))?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiSettingsError::Internal(InternalError::DbNotConnected));
    };
    let settings = load_settings(repository, &state.cipher).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_deref())))
}

async fn update_settings(
    State(state): State<RoiSettingsState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ROISettingsResponse>, RoiSettingsError> {
    write_admin(caller.as_ref().map(|extension| &extension.0))?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiSettingsError::Internal(InternalError::DbNotConnected));
    };
    let patch: ROISettingsUpdate = decode_body(&body)?;
    let presence: PatchPresence = decode_body(&body)?;
    let issues = request_issues(&patch);
    if !issues.is_empty() {
        return Err(RoiSettingsError::Request(issues));
    }
    let stored = load_stored(repository).await?;
    let current = load_settings(repository, &state.cipher).await?;
    if presence.github_api_url == Some(None) {
        return Err(RoiSettingsError::NullUrl);
    }
    let secrets = patched_secrets(&patch, &presence, &stored, &current, &state.cipher)?;
    let settings = build_settings(patch, &current, &secrets)?;
    save_settings(repository, &stored_from(&settings, &secrets)).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_deref())))
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
