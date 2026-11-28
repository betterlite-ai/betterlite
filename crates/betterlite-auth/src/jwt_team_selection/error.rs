use std::collections::BTreeSet;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_logging::analytics::python::{FormatError, repr};
use thiserror::Error;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_team_lookup::error::TeamLookupError;
use crate::jwt_claims::claims::ClaimError;

const FORBIDDEN: u16 = 403;

// The failures the JWT team selection raises, each carrying LiteLLM's detail text verbatim.
#[derive(Debug, Error)]
pub enum TeamSelectionError {
    #[error("No registered agent matches JWT claim {field}={claim}")]
    AgentNotRegistered { field: String, claim: String },
    #[error(transparent)]
    Lookup(Box<TeamLookupError>),
    #[error(transparent)]
    Claim(Box<ClaimError>),
    #[error(transparent)]
    Format(Box<FormatError>),
    #[error(
        "No team found in token. Checked team_id field '{team_id_field}' and team_alias field '{team_alias_field}'.{hint}"
    )]
    NoTeamInToken {
        team_id_field: String,
        team_alias_field: String,
        hint: String,
    },
    #[error(
        "No team has access to the requested model: {model}. Checked teams={checked}. Check `/models` to see all available models."
    )]
    NoTeamForModel { model: String, checked: String },
    #[error(
        "No teams found in token. `enforce_team_based_model_access` is set to True. Token must belong to a team."
    )]
    NoTeamsInToken,
    #[error(
        "Team not allowed to access passthrough route {route}. Configure `allowed_passthrough_routes` on the team."
    )]
    PassThroughDenied { route: String },
    #[error(
        "x-litellm-team-id '{header_value}' does not resolve to a team id or a unique team alias in your JWT's allowed teams. Allowed team ids: {allowed}"
    )]
    HeaderTeamNotAllowed {
        header_value: String,
        allowed: String,
    },
    #[error(
        "x-litellm-team-id '{header_value}' does not resolve to a team id or a unique team alias among your team memberships."
    )]
    HeaderTeamMembershipDenied { header_value: String },
    #[error(
        "No user or team id found in token. enforce_rbac is set to True. Token must belong to a proxy admin, team, or user."
    )]
    ObjectIdentityRequired,
}

impl TeamSelectionError {
    #[must_use]
    pub fn agent_not_registered(field: Option<&str>, claim: &str) -> Self {
        Self::AgentNotRegistered {
            field: field.unwrap_or("None").to_owned(),
            claim: claim.to_owned(),
        }
    }

    #[must_use]
    pub fn no_team_in_token(
        team_id_field: Option<&str>,
        team_alias_field: Option<&str>,
        hint: &str,
    ) -> Self {
        Self::NoTeamInToken {
            team_id_field: team_id_field.unwrap_or("None").to_owned(),
            team_alias_field: team_alias_field.unwrap_or("None").to_owned(),
            hint: hint.to_owned(),
        }
    }

    // sorted(ids) as Python prints them: ['a', 'b']
    pub fn header_team_not_allowed(
        header_value: String,
        allowed: &BTreeSet<String>,
    ) -> Result<Self, FormatError> {
        Ok(Self::HeaderTeamNotAllowed {
            header_value,
            allowed: format!("[{}]", quoted(allowed)?),
        })
    }

    // the set as Python prints it; betterlite sorts where LiteLLM keeps hash order
    pub fn no_team_for_model(model: &str, checked: &BTreeSet<String>) -> Result<Self, FormatError> {
        Ok(Self::NoTeamForModel {
            model: model.to_owned(),
            checked: format!("{{{}}}", quoted(checked)?),
        })
    }

    // B757's two renderings: an HTTPException keeps its status, any other exception becomes a 401
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        match self {
            Self::Lookup(error) => lookup_exception(&error),
            Self::Claim(error) => other_exception(error.to_string()),
            Self::Format(error) => other_exception(error.to_string()),
            // LiteLLM raises this one as a plain Exception, which becomes an auth error with a 401
            failure @ Self::NoTeamInToken { .. } => other_exception(failure.to_string()),
            failure @ (Self::AgentNotRegistered { .. }
            | Self::NoTeamForModel { .. }
            | Self::NoTeamsInToken
            | Self::PassThroughDenied { .. }
            | Self::HeaderTeamNotAllowed { .. }
            | Self::HeaderTeamMembershipDenied { .. }
            | Self::ObjectIdentityRequired) => forbidden_exception(failure.to_string()),
        }
    }
}

impl From<ClaimError> for TeamSelectionError {
    fn from(source: ClaimError) -> Self {
        Self::Claim(Box::new(source))
    }
}

impl From<FormatError> for TeamSelectionError {
    fn from(source: FormatError) -> Self {
        Self::Format(Box::new(source))
    }
}

fn lookup_exception(error: &TeamLookupError) -> ProxyException {
    if let Some((status_code, _detail)) = error.http() {
        let failure = AuthFailure::Http {
            detail: error.to_string(),
            param: None,
            status_code,
            invalid_virtual_key: false,
        };
        as_proxy_exception(&failure).exception().clone()
    } else {
        other_exception(error.to_string())
    }
}

fn forbidden_exception(message: String) -> ProxyException {
    let failure = AuthFailure::Http {
        detail: message,
        param: None,
        status_code: FORBIDDEN,
        invalid_virtual_key: false,
    };
    as_proxy_exception(&failure).exception().clone()
}

fn other_exception(message: String) -> ProxyException {
    let failure = AuthFailure::Other {
        message,
        param: None,
    };
    as_proxy_exception(&failure).exception().clone()
}

// Python's repr of each value, as the shared analytics formatter writes it
fn quoted(values: &BTreeSet<String>) -> Result<String, FormatError> {
    values
        .iter()
        .map(repr)
        .collect::<Result<Vec<_>, _>>()
        .map(|rendered| rendered.join(", "))
}
