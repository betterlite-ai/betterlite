use betterlite_auth::auth_identity::records::Principal;
use serde::Deserialize;

use crate::organization_create::caller::MASTER_KEY_SUBJECT;

const MASTER_KEY_ALIAS: &str = "litellm_proxy_master_key";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CallerKeyView {
    api_key: Option<String>,
}

// user_api_key_dict.api_key: LiteLLM's stable alias for the master key, else the canonical hash B2314 keeps
#[must_use = "the caller's key decides which session rows are visible"]
pub fn caller_api_key(principal: &Principal) -> Result<Option<String>, serde_json::Error> {
    if principal.subject == MASTER_KEY_SUBJECT {
        return Ok(Some(MASTER_KEY_ALIAS.to_owned()));
    }
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(None);
    };
    let view: CallerKeyView = serde_json::from_value(serde_json::to_value(key)?)?;
    Ok(view.api_key)
}
