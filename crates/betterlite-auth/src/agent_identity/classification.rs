use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use crate::agent_policy::{AgentExecutionMode, AgentIdentityBinding};
use crate::identity::{AgentIdentityFailure, AgentSubject, FailureCode, SubjectKind, SubjectMode};

pub fn classify_agent_subject(
    binding: &AgentIdentityBinding,
    claims: &Payload,
    allowed_mode: &AgentExecutionMode,
) -> Result<AgentSubject, AgentIdentityFailure> {
    let registered = claim_text(claims, "iss") == Some(binding.issuer.as_str())
        && claim_text(claims, "tid") == Some(binding.tenant_id.as_str())
        && claim_text(claims, "azp") == Some(binding.client_id.as_str());
    if !registered {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Token does not match the registered Entra application"),
        });
    }
    let Some(oid) = claim_text(claims, "oid").filter(|oid| !oid.is_empty()) else {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Entra token must identify its object subject"),
        });
    };
    reject_native_user_facet(claims)?;
    scope_subject(binding, claims, oid, allowed_mode)
}

fn claim_text<'claims>(claims: &'claims Payload, name: &str) -> Option<&'claims str> {
    match claims.0.get(name) {
        Some(JsonData::String(text)) => Some(text),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

fn reject_native_user_facet(claims: &Payload) -> Result<(), AgentIdentityFailure> {
    match claims.0.get("xms_sub_fct") {
        Some(JsonData::String(facets)) if !facets.split_whitespace().any(|facet| facet == "13") => {
            Ok(())
        }
        Some(JsonData::Null) | None => Ok(()),
        Some(
            JsonData::String(_)
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        ) => Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(
                "Native agent-user authentication is not supported by this binding",
            ),
        }),
    }
}

#[derive(Clone, Copy)]
enum TokenKind<'scope> {
    Delegated(&'scope str),
    Application,
}

impl TokenKind<'_> {
    const fn subject_kind(self) -> SubjectKind {
        match self {
            Self::Delegated(_) => SubjectKind::DelegatedSubject,
            Self::Application => SubjectKind::Application,
        }
    }

    const fn subject_mode(self) -> SubjectMode {
        match self {
            Self::Delegated(_) => SubjectMode::Delegated,
            Self::Application => SubjectMode::Autonomous,
        }
    }
}

fn scope_subject(
    binding: &AgentIdentityBinding,
    claims: &Payload,
    oid: &str,
    allowed_mode: &AgentExecutionMode,
) -> Result<AgentSubject, AgentIdentityFailure> {
    let kind = match claims.0.get("scp") {
        Some(JsonData::String(scope)) if !scope.is_empty() => TokenKind::Delegated(scope),
        Some(JsonData::String(_) | JsonData::Null) | None => TokenKind::Application,
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => {
            return Err(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from("Invalid delegated scope claim"),
            });
        }
    };
    if let Some(message) = contradiction(kind, binding, claims, oid, allowed_mode) {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(message),
        });
    }
    if let Some(message) = requirement_missing(kind, binding, claims)? {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(message),
        });
    }
    Ok(AgentSubject {
        kind: kind.subject_kind(),
        oid: oid.to_owned(),
        mode: kind.subject_mode(),
    })
}

fn contradiction(
    kind: TokenKind<'_>,
    binding: &AgentIdentityBinding,
    claims: &Payload,
    oid: &str,
    allowed_mode: &AgentExecutionMode,
) -> Option<&'static str> {
    let (rejected_mode, conflict_when_matched, rejected_idtyp, message) = match kind {
        TokenKind::Delegated(_) => (
            AgentExecutionMode::Autonomous,
            true,
            "app",
            "Delegated token contradicts the configured agent identity or mode",
        ),
        TokenKind::Application => (
            AgentExecutionMode::Delegated,
            false,
            "user",
            "Application token contradicts the configured agent identity or mode",
        ),
    };
    let principal_matches = binding.service_principal_id.as_deref() == Some(oid);
    let idtyp_conflict = claim_text(claims, "idtyp").is_some_and(|claim| claim == rejected_idtyp);
    if *allowed_mode == rejected_mode
        || principal_matches == conflict_when_matched
        || idtyp_conflict
    {
        return Some(message);
    }
    None
}

fn requirement_missing(
    kind: TokenKind<'_>,
    binding: &AgentIdentityBinding,
    claims: &Payload,
) -> Result<Option<&'static str>, AgentIdentityFailure> {
    match kind {
        TokenKind::Delegated(scope) => {
            let granted: BTreeSet<&str> = scope.split_whitespace().collect();
            let required: BTreeSet<&str> = binding
                .required_scopes
                .0
                .iter()
                .map(String::as_str)
                .collect();
            let missing = granted.is_empty() || !required.is_subset(&granted);
            Ok(missing.then_some("Token lacks the required delegated scopes"))
        }
        TokenKind::Application => {
            let roles = application_roles(claims)?;
            let missing = binding
                .required_roles
                .iter()
                .any(|required| !roles.contains(&required.as_str()));
            Ok(missing.then_some("Token lacks the required application roles"))
        }
    }
}

fn application_roles(claims: &Payload) -> Result<Vec<&str>, AgentIdentityFailure> {
    let Some(value) = claims.0.get("roles") else {
        return Ok(Vec::new());
    };
    let JsonData::Array(roles) = value else {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Invalid application roles claim"),
        });
    };
    let mut names = Vec::with_capacity(roles.len());
    for role in roles {
        let JsonData::String(name) = role else {
            return Err(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from("Invalid application roles claim"),
            });
        };
        names.push(name.as_str());
    }
    Ok(names)
}
