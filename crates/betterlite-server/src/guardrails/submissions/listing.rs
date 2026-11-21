use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use serde::Deserialize;

use super::SubmissionsState;
use super::error::{SubmissionError, logged};
use super::json_field::parse_json_field;
use super::models::{
    ACTIVE, DESCRIPTION, GuardrailSubmissionItem, GuardrailSubmissionSummary,
    ListGuardrailSubmissionsResponse, PENDING_REVIEW, REJECTED, SUBMITTED_BY_EMAIL,
    SUBMITTED_BY_USER_ID,
};
use super::rows::{ColumnJson, SubmissionRow, SubmissionRows, TeamFilter};
use super::teams::{authenticated, caller, has_admin_view, user_team_ids};
use crate::auth::auth_identity::AuthContext;
use crate::guardrails::admin_reads::masking::mask_sensitive_values;

const LIST_FAILED: &str = "Error listing guardrail submissions";
const GET_FAILED: &str = "Error getting guardrail submission";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct SubmissionQuery {
    status: Option<String>,
    team_id: Option<String>,
    search: Option<String>,
}

pub(super) async fn list_submissions<S: SubmissionRows + 'static>(
    State(state): State<Arc<SubmissionsState<S>>>,
    auth: Option<Extension<AuthContext>>,
    Query(query): Query<SubmissionQuery>,
) -> Result<Json<ListGuardrailSubmissionsResponse>, SubmissionError> {
    let context = authenticated(auth)?;
    let database = state.database.as_ref().ok_or(SubmissionError::NoDatabase)?;
    let visible = if has_admin_view(&context.principal) {
        None
    } else {
        let identity = caller(&context.principal)?;
        Some(user_team_ids(&identity, &state).await?)
    };
    if let (Some(ids), Some(team_id)) = (visible.as_ref(), query.team_id.as_ref())
        && !ids.contains(team_id)
    {
        return Err(SubmissionError::NotTeamMember {
            team_id: team_id.clone(),
        });
    }
    if visible.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Json(empty_answer()));
    }
    let filter = match visible {
        Some(ids) => TeamFilter::Teams(ids),
        None => TeamFilter::AnyTeam,
    };
    let rows = database
        .find_team_rows(filter)
        .await
        .map_err(|source| logged(LIST_FAILED, SubmissionError::Rows { source }))?;
    let summary = summarize(&rows);
    let submissions = rows
        .iter()
        .filter(|row| kept(row, &query))
        .map(item)
        .collect();
    Ok(Json(ListGuardrailSubmissionsResponse {
        submissions,
        summary,
    }))
}

pub(super) async fn get_submission<S: SubmissionRows + 'static>(
    State(state): State<Arc<SubmissionsState<S>>>,
    Path(guardrail_id): Path<String>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<GuardrailSubmissionItem>, SubmissionError> {
    let context = authenticated(auth)?;
    let database = state.database.as_ref().ok_or(SubmissionError::NoDatabase)?;
    let is_admin = has_admin_view(&context.principal);
    let row = database
        .find_by_id(&guardrail_id)
        .await
        .map_err(|source| logged(GET_FAILED, SubmissionError::Rows { source }))?
        .ok_or(SubmissionError::SubmissionNotFound)?;
    if !is_admin {
        let identity = caller(&context.principal)?;
        let visible = user_team_ids(&identity, &state)
            .await
            .map_err(|source| logged(GET_FAILED, source))?;
        let owned = match row.team_id.as_deref() {
            Some(team_id) => visible.iter().any(|id| id == team_id),
            None => false,
        };
        if !owned {
            return Err(SubmissionError::NotOwningTeam);
        }
    }
    Ok(Json(item(&row)))
}

fn empty_answer() -> ListGuardrailSubmissionsResponse {
    ListGuardrailSubmissionsResponse {
        submissions: Vec::new(),
        summary: GuardrailSubmissionSummary {
            total: 0,
            pending_review: 0,
            active: 0,
            rejected: 0,
        },
    }
}

// the summary counts every row of the query before the status, team and search filters
fn summarize(rows: &[SubmissionRow]) -> GuardrailSubmissionSummary {
    GuardrailSubmissionSummary {
        total: rows.len(),
        pending_review: rows
            .iter()
            .filter(|row| row_status(row) == PENDING_REVIEW)
            .count(),
        active: rows.iter().filter(|row| row_status(row) == ACTIVE).count(),
        rejected: rows
            .iter()
            .filter(|row| row_status(row) == REJECTED)
            .count(),
    }
}

fn row_status(row: &SubmissionRow) -> &str {
    match &row.status {
        Some(status) => status.as_str(),
        None => ACTIVE,
    }
}

fn kept(row: &SubmissionRow, query: &SubmissionQuery) -> bool {
    if let Some(status) = query.status.as_deref()
        && row.status.as_deref() != Some(status)
    {
        return false;
    }
    if let Some(team_id) = query.team_id.as_deref()
        && row.team_id.as_deref() != Some(team_id)
    {
        return false;
    }
    match query.search.as_deref() {
        Some(search) => search_matches(row, search),
        None => true,
    }
}

fn search_matches(row: &SubmissionRow, search: &str) -> bool {
    let needle = search.to_lowercase();
    if row.guardrail_name.to_lowercase().contains(needle.as_str()) {
        return true;
    }
    match &row.guardrail_info {
        Some(ColumnJson::Object(info)) => description_text(info)
            .to_lowercase()
            .contains(needle.as_str()),
        Some(ColumnJson::Text(text)) => text.to_lowercase().contains(needle.as_str()),
        None => false,
    }
}

// str((guardrail_info or {}).get("description", "")) for the one value type a description is
fn description_text(info: &JsonObject) -> &str {
    match info.get(DESCRIPTION) {
        Some(JsonValue::String(text)) => text,
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => "",
    }
}

// _row_to_submission_item: both JSON columns through _parse_json_field, params masked
pub(super) fn item(row: &SubmissionRow) -> GuardrailSubmissionItem {
    let info = parse_json_field(row.guardrail_info.as_ref()).unwrap_or_default();
    let params = parse_json_field(row.litellm_params.as_ref()).unwrap_or_default();
    let status = match &row.status {
        Some(status) => status.clone(),
        None => ACTIVE.to_owned(),
    };
    let submitted_by_user_id = info
        .get(SUBMITTED_BY_USER_ID)
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let submitted_by_email = info
        .get(SUBMITTED_BY_EMAIL)
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    GuardrailSubmissionItem {
        guardrail_id: row.guardrail_id.clone(),
        guardrail_name: row.guardrail_name.clone(),
        status,
        team_id: row.team_id.clone(),
        team_guardrail: row.team_id.is_some(),
        litellm_params: Some(mask_sensitive_values(&params)),
        guardrail_info: Some(info),
        submitted_by_user_id,
        submitted_by_email,
        submitted_at: row.submitted_at.map(Timestamp::utc),
        reviewed_at: row.reviewed_at.map(Timestamp::utc),
        created_at: row.created_at.map(Timestamp::utc),
        updated_at: row.updated_at.map(Timestamp::utc),
    }
}
