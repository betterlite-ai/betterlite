use betterlite_prompts::arguments::json::Json;

use crate::mcp_token_claims::Claims;

mod tests;

const SCOPE_LIMIT_CHARS: usize = 80;
const SCOPE_KEPT_CHARS: usize = 77;

#[must_use]
pub fn render(claims: &Claims, kid: &str) -> String {
    let sub = claims.get("sub").and_then(Json::as_str).unwrap_or_default();
    let iss = claims.get("iss").and_then(Json::as_str).unwrap_or_default();
    let exp = match claims.get("exp") {
        Some(Json::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    };
    let raw_scope = claims
        .get("scope")
        .and_then(Json::as_str)
        .unwrap_or_default();
    let scope = if raw_scope.chars().count() > SCOPE_LIMIT_CHARS {
        let kept: String = raw_scope.chars().take(SCOPE_KEPT_CHARS).collect();
        format!("{kept}...")
    } else {
        raw_scope.to_owned()
    };
    format!("v=1; kid={kid}; sub={sub}; iss={iss}; exp={exp}; scope={scope}")
}
