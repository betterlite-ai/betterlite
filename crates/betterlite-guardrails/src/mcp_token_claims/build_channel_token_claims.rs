use betterlite_prompts::arguments::json::Json;
use serde_json::Number;

use crate::mcp_token_claims::{ClaimError, Claims};

mod tests;

pub fn build(
    base_claims: &Claims,
    audience: &str,
    ttl_seconds: i64,
    now: i64,
) -> Result<Claims, ClaimError> {
    let expires = now
        .checked_add(ttl_seconds)
        .ok_or(ClaimError::TimestampOverflow)?;
    let mut claims = base_claims.clone();
    claims.insert("aud".to_owned(), Json::Text(audience.to_owned()));
    claims.insert("iat".to_owned(), Json::Number(Number::from(now)));
    claims.insert("exp".to_owned(), Json::Number(Number::from(expires)));
    claims.insert("nbf".to_owned(), Json::Number(Number::from(now)));
    Ok(claims)
}
