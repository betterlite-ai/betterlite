use serde::Serialize;

mod tests;

// LiteLLM's MIT jwt_display_template.py with betterlite branding; Berri AI's notice is in THIRD_PARTY_NOTICES.md
const TEMPLATE: &str = include_str!("jwt_display/template.html");

#[derive(Serialize)]
pub struct SsoDebugClaims<Parsed, Claims> {
    pub parsed_by_proxy: Parsed,
    pub raw_claims: Claims,
    pub access_token_claims: Claims,
}

pub fn render<Parsed, Claims>(
    claims: &SsoDebugClaims<Parsed, Claims>,
) -> Result<String, serde_json::Error>
where
    Parsed: Serialize,
    Claims: Serialize,
{
    let json = serde_json::to_string_pretty(claims)?;
    let mut statement = String::from("const ssoData = ");
    // JSON escapes keep claims as typed; HTML entities would change the text the page shows
    for character in json.chars() {
        match character {
            '<' => statement.push_str("\\u003c"),
            '>' => statement.push_str("\\u003e"),
            '&' => statement.push_str("\\u0026"),
            '\u{2028}' => statement.push_str("\\u2028"),
            '\u{2029}' => statement.push_str("\\u2029"),
            other => statement.push(other),
        }
    }
    statement.push(';');
    Ok(TEMPLATE.replacen("const ssoData = SSO_DATA;", &statement, 1))
}
