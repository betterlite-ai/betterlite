use betterlite_prompts::arguments::json::Json;

use crate::mcp_token_claims::{ClaimError, Claims};

mod tests;

pub fn validate(required: &[String], incoming: Option<&Claims>) -> Result<(), ClaimError> {
    let mut missing = Vec::new();
    for claim in required {
        let present = incoming
            .and_then(|claims| claims.get(claim.as_str()))
            .is_some_and(Json::is_truthy);
        if !present {
            missing.push(claim.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(ClaimError::MissingRequiredClaims { missing })
}
