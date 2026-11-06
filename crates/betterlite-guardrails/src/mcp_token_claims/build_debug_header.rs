use betterlite_prompts::arguments::json::Json;

use crate::mcp_token_claims::Claims;

mod tests;

const SCOPE_LIMIT_CHARS: usize = 80;
const SCOPE_KEPT_CHARS: usize = 77;

#[must_use]
pub fn render(claims: &Claims, kid: &str) -> String {
    let sub = claims.get("sub").map_or_else(String::new, claim_text);
    let iss = claims.get("iss").map_or_else(String::new, claim_text);
    let exp = claims.get("exp").map_or_else(|| "0".to_owned(), claim_text);
    let raw_scope = claims.get("scope").map_or_else(String::new, claim_text);
    let scope = if raw_scope.chars().count() > SCOPE_LIMIT_CHARS {
        let kept: String = raw_scope.chars().take(SCOPE_KEPT_CHARS).collect();
        format!("{kept}...")
    } else {
        raw_scope
    };
    format!("v=1; kid={kid}; sub={sub}; iss={iss}; exp={exp}; scope={scope}")
}

fn claim_text(value: &Json) -> String {
    match value {
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::Bool(flag) => flag.to_string(),
        Json::Null => "null".to_owned(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}
