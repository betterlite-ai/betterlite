use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use super::changes::Columns;
use super::changes::changed_fields as changed_columns;
use super::error::Forbidden;
use super::error::TeamAdminError;
use crate::team_request_contracts::shared::RequestProjection as _;
use crate::team_request_contracts::shared::to_object;
use crate::team_request_contracts::update::UpdateTeamRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamAdminEditVerdict {
    Allowed { request: UpdateTeamRequest },
    Disabled,
    FieldNotPermitted { field: String },
}

// Logical team fields whose stored value the request would change.
pub fn changed_fields(
    data: &UpdateTeamRequest,
    existing: &LiteLLMTeamTable,
) -> Result<BTreeSet<String>, TeamAdminError> {
    let JsonData::Object(submitted) = data.supplied()? else {
        return Err(TeamAdminError::NotAnObject);
    };
    let stored = to_object(existing)?;
    changed_columns(&submitted, &stored, Columns::Team)
}

// The request without the values it resends unchanged, which would otherwise still trigger derived writes.
fn only_changes(
    data: &UpdateTeamRequest,
    changed: &BTreeSet<String>,
) -> Result<UpdateTeamRequest, TeamAdminError> {
    let JsonData::Object(submitted) = data.supplied()? else {
        return Err(TeamAdminError::NotAnObject);
    };
    let mut kept: BTreeSet<&str> = BTreeSet::from(["team_id"]);
    for name in changed {
        if submitted.contains_key(name) {
            kept.insert(name.as_str());
        }
    }
    if changed.iter().any(|name| !submitted.contains_key(name)) {
        kept.insert("metadata");
    }
    let JsonData::Object(resolved) = data.resolved()? else {
        return Err(TeamAdminError::NotAnObject);
    };
    let restricted = resolved
        .into_iter()
        .filter(|(name, _)| kept.contains(name.as_str()))
        .collect();
    Ok(UpdateTeamRequest::from_value(&JsonData::Object(
        restricted,
    ))?)
}

pub fn edit_verdict(
    data: &UpdateTeamRequest,
    existing: &LiteLLMTeamTable,
    permitted: &BTreeSet<String>,
) -> Result<TeamAdminEditVerdict, TeamAdminError> {
    if permitted.is_empty() {
        return Ok(TeamAdminEditVerdict::Disabled);
    }
    let changed = changed_fields(data, existing)?;
    if let Some(field) = changed.iter().find(|name| !permitted.contains(*name)) {
        return Ok(TeamAdminEditVerdict::FieldNotPermitted {
            field: field.clone(),
        });
    }
    Ok(TeamAdminEditVerdict::Allowed {
        request: only_changes(data, &changed)?,
    })
}

pub fn request_or_raise(verdict: TeamAdminEditVerdict) -> Result<UpdateTeamRequest, Forbidden> {
    match verdict {
        TeamAdminEditVerdict::Allowed { request } => Ok(request),
        TeamAdminEditVerdict::Disabled => Err(Forbidden::EditingDisabled),
        TeamAdminEditVerdict::FieldNotPermitted { field } => {
            Err(Forbidden::FieldNotPermitted { field })
        }
    }
}
