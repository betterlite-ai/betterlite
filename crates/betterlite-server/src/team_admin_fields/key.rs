use std::collections::BTreeSet;

use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use super::KEY_BUDGET_FIELDS;
use super::changes::Columns;
use super::changes::changed_fields as changed_columns;
use super::error::Forbidden;
use super::error::TeamAdminError;
use crate::key_fields::update::UpdateKeyRequest;
use crate::team_request_contracts::shared::to_object;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamAdminKeyEditVerdict {
    Allowed { changed: BTreeSet<String> },
    Disabled,
    FieldNotPermitted { field: String },
}

// Logical key fields whose stored value the request would change.
pub fn changed_fields(
    data: &UpdateKeyRequest,
    existing: &LiteLLMVerificationToken,
) -> Result<BTreeSet<String>, TeamAdminError> {
    let supplied = data.project()?.supplied;
    let stored = to_object(existing)?;
    changed_columns(&supplied, &stored, Columns::Key)
}

pub fn edit_verdict(
    data: &UpdateKeyRequest,
    existing: &LiteLLMVerificationToken,
    enabled: bool,
) -> Result<TeamAdminKeyEditVerdict, TeamAdminError> {
    if !enabled {
        return Ok(TeamAdminKeyEditVerdict::Disabled);
    }
    let supplied = data.project()?.supplied;
    let stored = to_object(existing)?;
    let changed = changed_columns(&supplied, &stored, Columns::Key)?;
    let mut blocked: BTreeSet<&str> = changed
        .iter()
        .map(String::as_str)
        .filter(|name| !KEY_BUDGET_FIELDS.contains(name))
        .collect();
    if supplied.contains_key("spend") {
        blocked.insert("spend");
    }
    match blocked.first() {
        Some(field) => Ok(TeamAdminKeyEditVerdict::FieldNotPermitted {
            field: (*field).to_owned(),
        }),
        None => Ok(TeamAdminKeyEditVerdict::Allowed { changed }),
    }
}

pub fn request_or_raise(verdict: TeamAdminKeyEditVerdict) -> Result<(), Forbidden> {
    match verdict {
        TeamAdminKeyEditVerdict::Allowed { .. } => Ok(()),
        TeamAdminKeyEditVerdict::Disabled => Err(Forbidden::MemberKeyEditingDisabled),
        TeamAdminKeyEditVerdict::FieldNotPermitted { field } => {
            Err(Forbidden::KeyFieldNotPermitted { field })
        }
    }
}
