use betterlite_paths::paths::PathLimits;
use betterlite_paths::paths::lookup::find;
use thiserror::Error;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::ClaimError;

// jwt_claims' CLAIM_PATH_LIMITS by hand: widening it needs B760's file, outside B1609's scope
const CLAIM_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 8192,
    max_depth: 256,
    max_selectors: 256,
    max_visits: 100_000,
};

#[derive(Debug, Error)]
pub enum SsoClaimsError {
    #[error(transparent)]
    Claim(#[from] ClaimError),
}

// JwtClaims::nested carries a handler's settings; this reads the same dot path over a plain response
pub(super) fn present(response: &TypedValue, path: &str) -> Result<Option<TypedValue>, ClaimError> {
    find(&CLAIM_PATH_LIMITS, response, path)
        .map_err(|source| ClaimError::Path {
            claim: path.to_owned(),
            source,
        })
        .map(|found| found.filter(|value| !value.json.is_null()))
}

// a claim that does not hold its declared type, named by its path
pub(super) fn type_error(path: &str, source: serde_json::Error) -> ClaimError {
    ClaimError::Type {
        claim: path.to_owned(),
        source,
    }
}

// nested_text over a plain response: null is absent, another type is an error naming the path
pub(super) fn claim_text(response: &TypedValue, path: &str) -> Result<Option<String>, ClaimError> {
    let Some(found) = present(response, path)? else {
        return Ok(None);
    };
    let text = serde_json::from_value(found.json).map_err(|source| type_error(path, source))?;
    Ok(Some(text))
}

// a group claim: a list contributes its string members, a comma-separated string splits and trims, anything else has none
pub(super) fn claim_groups(response: &TypedValue, path: &str) -> Result<Vec<String>, ClaimError> {
    let Some(found) = present(response, path)? else {
        return Ok(Vec::new());
    };
    if let Some(members) = found.json.as_array() {
        return Ok(members
            .iter()
            .filter_map(|member| member.as_str())
            .map(str::to_owned)
            .collect());
    }
    Ok(found
        .json
        .as_str()
        .into_iter()
        .flat_map(|text| text.split(','))
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_owned)
        .collect())
}
