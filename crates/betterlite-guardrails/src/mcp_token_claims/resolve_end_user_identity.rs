use betterlite_prompts::arguments::json::Json;
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};

use crate::mcp_token_claims::{AuthFields, Claims};

mod tests;

const API_KEY_PREFIX: &str = "apikey:";
const HASH_HEX_CHARS: usize = 16;
const PROXY_SUBJECT: &str = "litellm-proxy";

pub fn resolve(sources: &[String], auth: &AuthFields, incoming: Option<&Claims>) -> String {
    for source in sources {
        let candidate = if let Some(claim) = source.strip_prefix("token:") {
            incoming
                .and_then(|claims| claims.get(claim))
                .and_then(claim_text)
        } else {
            match source.as_str() {
                "litellm:user_id" => auth.user_id.clone(),
                "litellm:email" => auth.user_email.clone(),
                "litellm:end_user_id" => auth.end_user_id.clone(),
                "litellm:team_id" => auth.team_id.clone(),
                _ => {
                    tracing::warn!(%source, "unknown end-user claim source, skipping");
                    continue;
                }
            }
        };
        if let Some(selected) = candidate.filter(|text| !text.is_empty()) {
            return selected;
        }
    }
    let credential = auth
        .token
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
        .or_else(|| {
            auth.api_key
                .as_ref()
                .filter(|secret| !secret.expose_secret().is_empty())
        });
    let Some(secret) = credential else {
        return PROXY_SUBJECT.to_owned();
    };
    let digest = hex::encode(Sha256::digest(secret.expose_secret().as_bytes()));
    let prefix: String = digest.chars().take(HASH_HEX_CHARS).collect();
    format!("{API_KEY_PREFIX}{prefix}")
}

fn claim_text(value: &Json) -> Option<String> {
    if !value.is_truthy() {
        return None;
    }
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Bool(flag) => Some(flag.to_string()),
        // a JWT sub is a StringOrURI, and Python's list text is a repr this project does not port
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}
