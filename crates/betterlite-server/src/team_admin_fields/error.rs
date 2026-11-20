use thiserror::Error;

use crate::key_fields::error::DecodeError;
use crate::team_request_contracts::TeamContractError;

// The four policy rejections, each with LiteLLM's exact 403 message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Forbidden {
    #[error(
        "Team admins on this proxy cannot edit team settings. Ask a proxy admin to enable fields under Settings > UI > Team admin editable fields."
    )]
    EditingDisabled,
    #[error(
        "Team admins on this proxy do not have permission to update '{field}'. Ask a proxy admin to add it under Settings > UI > Team admin editable fields."
    )]
    FieldNotPermitted { field: String },
    #[error(
        "Team admins on this proxy cannot update budgets on other members' keys. Ask a proxy admin to enable 'member_key_budgets' under Settings > UI > Team admin editable fields."
    )]
    MemberKeyEditingDisabled,
    #[error(
        "Team admins on this proxy may only update budget fields on other members' keys, not '{field}'. Ask a proxy admin to add it under Settings > UI > Team admin editable fields."
    )]
    KeyFieldNotPermitted { field: String },
}

#[derive(Debug, Error)]
pub enum TeamAdminError {
    #[error("the team contract rejected the JSON value: {source}")]
    Contract {
        #[from]
        source: TeamContractError,
    },
    #[error("the key request does not decode: {source}")]
    Key {
        #[from]
        source: DecodeError,
    },
    #[error("the projected request is not a JSON object")]
    NotAnObject,
    #[error("a budget window value is not hashable, so the window pair cannot be compared")]
    UnhashableWindow,
}
