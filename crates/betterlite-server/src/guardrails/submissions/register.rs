use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_db::budget_permission_rows::Timestamp;
use chrono::{DateTime, Utc};
use url::Url;

use super::SubmissionsState;
use super::error::{SubmissionError, logged};
use super::models::{
    API_BASE, GENERIC_GUARDRAIL_API, GUARDRAIL, MODE, PENDING_REVIEW, RegisterGuardrailRequest,
    RegisterGuardrailResponse, SUBMITTED_BY_EMAIL, SUBMITTED_BY_USER_ID, TEAM_GUARDRAIL,
    decode_register_body,
};
use super::rows::{NewSubmission, SubmissionRow, SubmissionRows};
use super::teams::{Caller, authenticated, caller, user_team_ids};
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::PROXY_ADMIN_ROLE;

const UNIQUENESS_FAILED: &str = "Error checking guardrail name uniqueness";
const REGISTER_FAILED: &str = "Error registering guardrail";
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";

pub(super) async fn register_guardrail<S>(
    State(state): State<Arc<SubmissionsState<S>>>,
    auth: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<RegisterGuardrailResponse>, SubmissionError>
where
    S: SubmissionRows + 'static,
{
    let context = authenticated(auth)?;
    let request = decode_register_body(&body)?;
    let database = state
        .database
        .as_deref()
        .ok_or(SubmissionError::NoDatabase)?;
    let identity = caller(&context.principal)?;
    let team_id = choose_team(&request, &identity)?;
    ensure_membership(&identity, &team_id, &state).await?;
    validate_params(&request.litellm_params)?;
    let now = state.clock.now();
    let created = submit(database, request, &identity, &team_id, now).await?;
    let status = created.status.unwrap_or_default();
    Ok(Json(RegisterGuardrailResponse {
        guardrail_id: created.guardrail_id,
        guardrail_name: created.guardrail_name,
        status,
        submitted_at: created.submitted_at.map(Timestamp::utc),
    }))
}

// team_id = request.team_id or user_api_key_dict.team_id, where an empty string counts as absent
fn choose_team(
    request: &RegisterGuardrailRequest,
    identity: &Caller,
) -> Result<String, SubmissionError> {
    request
        .team_id
        .as_deref()
        .filter(|team| !team.is_empty())
        .or_else(|| identity.team_id.as_deref().filter(|team| !team.is_empty()))
        .map(str::to_owned)
        .ok_or(SubmissionError::TeamRequired)
}

// only a body team the caller's key is not bound to needs the lookup, and proxy_admin skips it
async fn ensure_membership<S>(
    identity: &Caller,
    team_id: &str,
    state: &SubmissionsState<S>,
) -> Result<(), SubmissionError>
where
    S: SubmissionRows,
{
    if identity.role.as_deref() == Some(PROXY_ADMIN_ROLE) {
        return Ok(());
    }
    if identity.team_id.as_deref() == Some(team_id) {
        return Ok(());
    }
    let teams = user_team_ids(identity, state).await?;
    if teams.iter().any(|id| id == team_id) {
        return Ok(());
    }
    Err(SubmissionError::NotTeamMember {
        team_id: team_id.to_owned(),
    })
}

fn validate_params(params: &BTreeMap<String, GuardrailValue>) -> Result<(), SubmissionError> {
    let is_generic = matches!(
        params.get(GUARDRAIL),
        Some(GuardrailValue::Text(name)) if name == GENERIC_GUARDRAIL_API
    );
    if !is_generic {
        return Err(SubmissionError::GuardrailType);
    }
    let Some(api_base) = params.get(API_BASE) else {
        return Err(SubmissionError::ApiBaseRequired);
    };
    let api_base_text = match api_base {
        GuardrailValue::Null => return Err(SubmissionError::ApiBaseRequired),
        GuardrailValue::Text(text) if text.is_empty() => {
            return Err(SubmissionError::ApiBaseRequired);
        }
        GuardrailValue::Text(text) => text.as_str(),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => return Err(SubmissionError::ApiBaseScheme),
    };
    let scheme = scheme_of(api_base_text);
    if !scheme.eq_ignore_ascii_case(HTTP_SCHEME) && !scheme.eq_ignore_ascii_case(HTTPS_SCHEME) {
        return Err(SubmissionError::ApiBaseScheme);
    }
    let has_host = Url::parse(api_base_text)
        .is_ok_and(|url| url.host_str().is_some_and(|host| !host.is_empty()));
    if !has_host {
        return Err(SubmissionError::ApiBaseHost);
    }
    if matches!(params.get(MODE), None | Some(GuardrailValue::Null)) {
        return Err(SubmissionError::ModeRequired);
    }
    Ok(())
}

// urlparse's scheme: the text before the first colon, which only http and https may be
fn scheme_of(api_base: &str) -> &str {
    api_base
        .split_once(':')
        .map_or("", |(scheme, _rest)| scheme)
}

async fn submit<S>(
    database: &S,
    request: RegisterGuardrailRequest,
    identity: &Caller,
    team_id: &str,
    now: DateTime<Utc>,
) -> Result<SubmissionRow, SubmissionError>
where
    S: SubmissionRows,
{
    let existing = database
        .find_by_name(&request.guardrail_name)
        .await
        .map_err(|source| logged(UNIQUENESS_FAILED, SubmissionError::Rows { source }))?;
    if existing.is_some() {
        return Err(SubmissionError::DuplicateName {
            name: request.guardrail_name,
        });
    }
    let params_text = serde_json::to_string(&request.litellm_params)
        .map_err(|source| SubmissionError::Encode { source })?;
    let info_text = serde_json::to_string(&submission_info(&request, identity))
        .map_err(|source| SubmissionError::Encode { source })?;
    database
        .create(NewSubmission {
            guardrail_name: request.guardrail_name,
            litellm_params: params_text,
            guardrail_info: info_text,
            status: PENDING_REVIEW.to_owned(),
            team_id: team_id.to_owned(),
            submitted_at: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .map_err(|source| logged(REGISTER_FAILED, SubmissionError::Rows { source }))
}

// dict(request.guardrail_info or {}) plus the submitter and the team marker
fn submission_info(
    request: &RegisterGuardrailRequest,
    identity: &Caller,
) -> BTreeMap<String, GuardrailValue> {
    let mut info = request.guardrail_info.clone().unwrap_or_default();
    let user_id = identity
        .user_id
        .as_deref()
        .map(str::to_owned)
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    let email = identity
        .user_email
        .as_deref()
        .map(str::to_owned)
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    info.insert(SUBMITTED_BY_USER_ID.to_owned(), user_id);
    info.insert(SUBMITTED_BY_EMAIL.to_owned(), email);
    info.insert(TEAM_GUARDRAIL.to_owned(), GuardrailValue::Boolean(true));
    info
}
