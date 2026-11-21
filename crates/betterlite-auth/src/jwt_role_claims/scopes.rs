use betterlite_paths::paths::Document;

use super::error::{RoleClaimError, kind_name};
use crate::jwt_claims::TypedValue;

const SCOPE_CLAIM: &str = "scope";

// JWTHandler.get_scopes reads the literal scope claim, no dot path; a list keeps every member as sent
pub fn scopes(claims: &TypedValue) -> Result<Vec<TypedValue>, RoleClaimError> {
    let Some(scope) = claims.json.get(SCOPE_CLAIM) else {
        return Ok(Vec::new());
    };
    if let Some(items) = scope.as_array() {
        let members = items.iter().map(|item| Document { json: item.clone() });
        return Ok(members.collect());
    }
    let Some(text) = scope.as_str() else {
        let found = kind_name(&Document {
            json: scope.clone(),
        });
        return Err(RoleClaimError::ScopeType { found });
    };
    // str.split() also breaks on the four ASCII separators U+001C..U+001F, which Rust's whitespace leaves out
    let words = text
        .split(|letter: char| letter.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&letter))
        .filter(|word| !word.is_empty());
    Ok(words
        .map(|word| Document {
            json: serde_json::json!(word),
        })
        .collect())
}
