use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use crate::agent_policy::{AgentExecutionMode, AgentIdentityBinding};
use crate::identity::{AgentIdentityFailure, AgentSubject, FailureCode, SubjectKind, SubjectMode};

pub fn classify_agent_subject(
    binding: &AgentIdentityBinding,
    claims: &Payload,
    allowed_mode: &AgentExecutionMode,
) -> Result<AgentSubject, AgentIdentityFailure> {
    let issuer_matches = claim_text(claims, "iss") == Some(binding.issuer.as_str());
    let tenant_matches = claim_text(claims, "tid") == Some(binding.tenant_id.as_str());
    let client_matches = claim_text(claims, "azp") == Some(binding.client_id.as_str());
    if !issuer_matches || !tenant_matches || !client_matches {
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
    match claims.0.get("scp") {
        Some(JsonData::String(scope)) if !scope.is_empty() => {
            delegated_subject(binding, claims, oid, scope, allowed_mode)
        }
        Some(JsonData::String(_) | JsonData::Null) | None => {
            application_subject(binding, claims, oid, allowed_mode)
        }
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Invalid delegated scope claim"),
        }),
    }
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

fn delegated_subject(
    binding: &AgentIdentityBinding,
    claims: &Payload,
    oid: &str,
    scope: &str,
    allowed_mode: &AgentExecutionMode,
) -> Result<AgentSubject, AgentIdentityFailure> {
    if *allowed_mode == AgentExecutionMode::Autonomous
        || binding.service_principal_id.as_deref() == Some(oid)
        || claim_text(claims, "idtyp") == Some("app")
    {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(
                "Delegated token contradicts the configured agent identity or mode",
            ),
        });
    }
    let granted: Vec<&str> = scope.split_whitespace().collect();
    let covers_required_scopes = binding
        .required_scopes
        .0
        .iter()
        .all(|required| granted.contains(&required.as_str()));
    if granted.is_empty() || !covers_required_scopes {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Token lacks the required delegated scopes"),
        });
    }
    Ok(AgentSubject {
        kind: SubjectKind::DelegatedSubject,
        oid: oid.to_owned(),
        mode: SubjectMode::Delegated,
    })
}

fn application_subject(
    binding: &AgentIdentityBinding,
    claims: &Payload,
    oid: &str,
    allowed_mode: &AgentExecutionMode,
) -> Result<AgentSubject, AgentIdentityFailure> {
    if *allowed_mode == AgentExecutionMode::Delegated
        || binding.service_principal_id.as_deref() != Some(oid)
        || claim_text(claims, "idtyp") == Some("user")
    {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(
                "Application token contradicts the configured agent identity or mode",
            ),
        });
    }
    let roles = application_roles(claims)?;
    let covers_required_roles = binding
        .required_roles
        .iter()
        .all(|required| roles.contains(&required.as_str()));
    if !covers_required_roles {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Token lacks the required application roles"),
        });
    }
    Ok(AgentSubject {
        kind: SubjectKind::Application,
        oid: oid.to_owned(),
        mode: SubjectMode::Autonomous,
    })
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
