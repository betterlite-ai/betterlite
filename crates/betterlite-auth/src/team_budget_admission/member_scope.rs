use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::ExceptionCode;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

use crate::auth_model_patterns::decision::can_object_call_model;
use crate::auth_model_patterns::denial::{ModelAccessDecision, model_denied_client_message};
use crate::auth_model_patterns::grant::{ModelGrantPattern, key_model_aliases};
use crate::auth_request_models::RequestedModels;

use super::error::TeamBudgetError;
use super::sources::{member_budget_row, resolve_membership};
use super::{TeamAdmission, TeamRequest};

// _check_team_member_model_access: only a nonempty member allowlist restricts; B747's team denial becomes the member denial
pub async fn check_team_member_model_access(
    admission: &TeamAdmission<'_>,
    request: &TeamRequest<'_>,
) -> Result<(), TeamBudgetError> {
    let (Some(team), Some(token)) = (request.team, request.token) else {
        return Ok(());
    };
    let Some(user_id) = token.view.token.user_id.as_deref() else {
        return Ok(());
    };
    let team_id = team.fields.base.team_id.as_str();
    let membership =
        resolve_membership(admission.memberships, &request.membership, user_id, team_id).await?;
    let Some(allowed) = membership
        .as_ref()
        .and_then(member_budget_row)
        .and_then(|row| row.allowed_models.as_ref())
        .filter(|models| !models.is_empty())
    else {
        return Ok(());
    };
    let grant = ModelGrantPattern {
        models: allowed.clone(),
        team_model_aliases: BTreeMap::new(),
        key_model_aliases: key_model_aliases(&token.view.token.aliases.0),
        team_id: Some(team_id.to_owned()),
    };
    match can_object_call_model(
        request.model,
        &grant,
        ModelAccessObject::Team,
        admission.models,
        0,
    ) {
        ModelAccessDecision::Allowed => Ok(()),
        ModelAccessDecision::Denied(_) => {
            Err(member_denial(user_id, team_id, request.model, allowed))
        }
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            Err(TeamBudgetError::RecurseDepth { model })
        }
    }
}

// the client keeps B747's safe message; the user, team and allowlist stay in the internal message
fn member_denial(
    user_id: &str,
    team_id: &str,
    model: &RequestedModels,
    allowed: &[String],
) -> TeamBudgetError {
    let requested = requested_text(model);
    let internal = format!(
        "Team member not allowed to access model. User={user_id}, Team={team_id}, Model={requested}. Allowed member models = {allowed:?}"
    );
    // ast-grep-ignore: no-long-names
    TeamBudgetError::ModelDenied(Box::new(ModelAccessDeniedProxyException::new(
        model_denied_client_message(&requested),
        internal,
        ProxyErrorTypes::TeamModelAccessDenied.to_string(),
        Some("model".to_owned()),
        ExceptionCode::Status(403),
    )))
}

// the model the request named, as the denial messages spell it
fn requested_text(model: &RequestedModels) -> String {
    match model {
        RequestedModels::None => String::new(),
        RequestedModels::One(name) => name.clone(),
        RequestedModels::Many(names) => names.join(","),
    }
}
