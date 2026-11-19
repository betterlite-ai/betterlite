use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_roi::records::ROIReport;
use betterlite_roi::settings::{normalize_email, normalize_login};
use betterlite_roi::summary::identity_map::{ROIIdentityMapResponse, ROIIdentityMapUpdate};
use betterlite_roi::summary::summarize;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::roi_connections::access::{WRITE_FORBIDDEN, WRITE_ROLES, ensure_admin};
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::{REPORT_KEY, load_parameter, load_settings};

use super::decode::decode_body;
use super::error::{RoiRoutesError, issue};
use super::state::RoiRouteState;
use super::stored::{load_ciphertexts, save_settings};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityBody {
    github_login: String,
    #[serde(deserialize_with = "Option::deserialize")]
    email: Option<String>,
}

pub fn parse_identity_body(body: &[u8]) -> Result<ROIIdentityMapUpdate, RoiRoutesError> {
    let payload: IdentityBody = decode_body(body)?;
    let github_login = normalized_login(&payload.github_login)?;
    Ok(ROIIdentityMapUpdate {
        github_login,
        email: payload.email,
    })
}

pub(super) async fn update_identity_map(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ROIIdentityMapResponse>, RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, WRITE_ROLES, WRITE_FORBIDDEN)?;
    let Some(repository) = state.repository.clone() else {
        return Err(RoiRoutesError::DbNotConnected);
    };
    let payload = parse_identity_body(&body)?;
    let mut settings = load_settings(repository.as_ref(), state.cipher.as_ref()).await?;
    let stored = load_ciphertexts(repository.as_ref()).await?;
    let new_email = payload
        .email
        .as_deref()
        .map_or_else(String::new, normalize_email);
    if payload.email.is_some() && new_email.is_empty() {
        return Err(RoiRoutesError::InvalidIdentity);
    }
    let identity_map = &mut settings.identity_map;
    if payload.email.is_some() {
        identity_map.insert(payload.github_login, new_email);
    } else {
        identity_map.shift_remove(payload.github_login.as_str());
    }
    save_settings(repository.as_ref(), &settings, &stored).await?;
    let stored_report = load_parameter::<ROIReport, _>(
        repository.as_ref(),
        REPORT_KEY,
        RoiSettingsError::StoredReport,
    )
    .await?;
    let summary = stored_report
        .as_ref()
        .map(|report| summarize(report, &settings.identity_map))
        .transpose()?;
    Ok(Json(ROIIdentityMapResponse {
        report: summary,
        identity_map: settings.identity_map,
    }))
}

fn normalized_login(raw: &str) -> Result<String, RoiRoutesError> {
    normalize_login(raw).map_err(|error| {
        RoiRoutesError::Request(vec![issue(
            "value_error",
            vec!["body", "github_login"],
            &format!("Value error, {error}"),
        )])
    })
}
