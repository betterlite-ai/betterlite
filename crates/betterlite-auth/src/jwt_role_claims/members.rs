use serde_json::json;

use super::error::{RoleClaimError, kind_name};
use crate::jwt_claims::TypedValue;

// one item of Python's iteration over a role claim; only text can equal a configured role
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    Text(String),
    Scalar,
    Container,
}

// a string yields its characters, a list its items and an object its keys, as iter() does
pub fn members(claim: &TypedValue) -> Result<Vec<Member>, RoleClaimError> {
    let json = &claim.json;
    if let Some(text) = json.as_str() {
        return Ok(text
            .chars()
            .map(|letter| Member::Text(String::from(letter)))
            .collect());
    }
    if let Some(entries) = json.as_object() {
        return Ok(entries.keys().cloned().map(Member::Text).collect());
    }
    let Some(items) = json.as_array() else {
        let found = kind_name(claim);
        return Err(RoleClaimError::NotIterable { found });
    };
    let members = items.iter().map(|item| {
        if let Some(text) = item.as_str() {
            Member::Text(text.to_owned())
        } else if item.is_array() || item.is_object() {
            Member::Container
        } else {
            Member::Scalar
        }
    });
    Ok(members.collect())
}

// Python's falsy claim values: null, false, zero and the empty string, list and object
#[must_use]
pub fn is_empty_claim(claim: &TypedValue) -> bool {
    claim.json.as_f64().is_some_and(|number| number == 0.0_f64)
        || [json!(null), json!(false), json!(""), json!([]), json!({})].contains(&claim.json)
}
