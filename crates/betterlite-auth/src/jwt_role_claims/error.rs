use betterlite_paths::error::PathError;
use thiserror::Error;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::ClaimError;

#[derive(Debug, Error)]
pub enum RoleClaimError {
    #[error("the scope claim holds {found}; a scope claim is a list or a string")]
    ScopeType { found: &'static str },
    #[error("the role claim holds {found}, which has no members to read")]
    NotIterable { found: &'static str },
    #[error("the role claim lists a list or an object, which cannot be a role")]
    Unhashable,
    #[error("the role claim lists {found}, which no role pattern can match")]
    NotText { found: &'static str },
    #[error("the role claim path {claim:?} is outside the dot-path limits")]
    Path {
        claim: String,
        #[source]
        source: PathError,
    },
    #[error("an identity claim of the token could not be read")]
    Identity(#[source] ClaimError),
}

pub(super) fn kind_name(value: &TypedValue) -> &'static str {
    let json = &value.json;
    if json.is_null() {
        "null"
    } else if json.is_boolean() {
        "a boolean"
    } else if json.is_number() {
        "a number"
    } else if json.is_string() {
        "a string"
    } else if json.is_array() {
        "a list"
    } else {
        "an object"
    }
}
