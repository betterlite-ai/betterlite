use axum::Json;
use axum::extract::{Extension, Query, State};
use betterlite_roi::records::ROIReport;
use betterlite_roi::sample::sample_report;
use betterlite_roi::summary::models::ROIReportResponse;
use betterlite_roi::summary::summarize;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::roi_connections::access::{READ_FORBIDDEN, READ_ROLES, ensure_admin};
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::{REPORT_KEY, load_parameter, load_settings};

use super::error::{MODE_LITERAL, RoiRoutesError, issue};
use super::state::RoiRouteState;

#[derive(Deserialize)]
pub(super) struct ReportQuery {
    #[serde(default)]
    mode: Option<String>,
}

pub(super) async fn read_report(
    State(state): State<RoiRouteState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<ReportQuery>,
) -> Result<Json<ROIReportResponse>, RoiRoutesError> {
    let context = caller.as_ref().map(|extension| &extension.0);
    ensure_admin(context, READ_ROLES, READ_FORBIDDEN)?;
    let Some(repository) = state.repository.as_deref() else {
        return Err(RoiRoutesError::DbNotConnected);
    };
    match query.mode.as_deref().unwrap_or("live") {
        "demo" => {
            let summary = summarize(&sample_report((state.clock)()), &IndexMap::new())?;
            Ok(Json(ROIReportResponse {
                report: Some(summary),
            }))
        }
        "live" => {
            let stored_report = load_parameter::<ROIReport, _>(
                repository,
                REPORT_KEY,
                RoiSettingsError::StoredReport,
            )
            .await?;
            let Some(report) = stored_report else {
                return Ok(Json(ROIReportResponse { report: None }));
            };
            let settings = load_settings(repository, state.cipher.as_ref()).await?;
            let summary = summarize(&report, &settings.identity_map)?;
            Ok(Json(ROIReportResponse {
                report: Some(summary),
            }))
        }
        _ => Err(RoiRoutesError::Request(vec![issue(
            "literal_error",
            vec!["query", "mode"],
            MODE_LITERAL,
        )])),
    }
}
