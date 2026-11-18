use thiserror::Error;

use super::literal::hashable::has_unhashable_member;
use super::literal::{Literal, LiteralError, parse_literal};
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RoleMappingError {
    #[error("the role mapping is not a Python literal: {source}")]
    Literal {
        #[from]
        source: LiteralError,
    },
    #[error("the role mapping key is not a role name")]
    KeyNotRole,
    #[error("the groups mapped to {role} are not a list of group names")]
    GroupsNotText { role: UserRole },
    #[error("{role} is not a LiteLLM role")]
    UnknownRole { role: String },
    #[error("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM is not set, so the mapping has no group claim")]
    MissingGroupClaim,
    #[error("{role} is not a LiteLLM role")]
    UnknownDefaultRole { role: String },
}

// Only a dictionary literal overrides the stored mapping; unhashable keys and non-dicts fall back
pub fn decode_role_mapping_literal(
    roles: &[u8],
    group_claim: Option<&str>,
    default_role: Option<&str>,
) -> Result<Option<RoleMappings>, RoleMappingError> {
    let literal = parse_literal(roles)?;
    if has_unhashable_member(&literal) {
        return Ok(None);
    }
    let Literal::Dict(entries) = literal else {
        return Ok(None);
    };
    let mapped = literal_roles(entries)?;
    let Some(claim) = group_claim else {
        return Err(RoleMappingError::MissingGroupClaim);
    };
    Ok(Some(RoleMappings {
        provider: "generic".to_owned(),
        group_claim: claim.to_owned(),
        default_role: parse_default_role(default_role)?,
        roles: mapped,
    }))
}

fn literal_roles(
    entries: Vec<(Literal, Literal)>,
) -> Result<Vec<(UserRole, Vec<String>)>, RoleMappingError> {
    let mut mapped = Vec::new();
    for (key, value) in entries {
        let Literal::Str(name) = key else {
            return Err(RoleMappingError::KeyNotRole);
        };
        let role = name
            .parse::<UserRole>()
            .map_err(|_source| RoleMappingError::UnknownRole { role: name })?;
        mapped.push((role, literal_groups(role, value)?));
    }
    Ok(mapped)
}

fn literal_groups(role: UserRole, value: Literal) -> Result<Vec<String>, RoleMappingError> {
    let Literal::List(items) = value else {
        return Err(RoleMappingError::GroupsNotText { role });
    };
    let mut groups = Vec::new();
    for item in items {
        let Literal::Str(group) = item else {
            return Err(RoleMappingError::GroupsNotText { role });
        };
        groups.push(group);
    }
    Ok(groups)
}

fn parse_default_role(default_role: Option<&str>) -> Result<Option<UserRole>, RoleMappingError> {
    let Some(text) = default_role else {
        return Ok(None);
    };
    let role =
        text.parse::<UserRole>()
            .map_err(|_source| RoleMappingError::UnknownDefaultRole {
                role: text.to_owned(),
            })?;
    Ok(Some(role))
}
