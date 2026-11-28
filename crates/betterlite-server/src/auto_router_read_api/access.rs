use betterlite_auth::auth_identity::records::Principal;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CallerKeyView {
    #[serde(rename = "api_key")]
    api_key_hash: Option<String>,
}

// user_api_key_dict.api_key: the key's logged api_key, the master key's alias included
#[must_use = "the caller's key decides which session rows are visible"]
pub fn caller_api_key(principal: &Principal) -> Result<Option<String>, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(None);
    };
    let view: CallerKeyView = serde_json::from_value(serde_json::to_value(key)?)?;
    Ok(view.api_key_hash)
}
