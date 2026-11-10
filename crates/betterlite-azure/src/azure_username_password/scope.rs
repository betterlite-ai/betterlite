use std::collections::BTreeSet;

use super::UsernamePasswordError;

pub const RESERVED_SCOPES: [&str; 3] = ["offline_access", "openid", "profile"];

pub fn decorate(scope: &str) -> Result<String, UsernamePasswordError> {
    if RESERVED_SCOPES.contains(&scope) {
        return Err(UsernamePasswordError::ReservedScope);
    }
    let mut scopes: BTreeSet<&str> = RESERVED_SCOPES.into_iter().collect();
    scopes.insert(scope);
    Ok(scopes.into_iter().collect::<Vec<&str>>().join(" "))
}
