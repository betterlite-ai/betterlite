use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::error::PillarError;
use super::settings::PillarSettings;

// The fields of LiteLLM's UserAPIKeyAuth that Pillar forwards; litellm_params_metadata stays out
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyContext {
    pub key_name: Option<String>,
    pub key_alias: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub org_id: Option<String>,
}

fn set_bool_header(headers: &mut BTreeMap<String, String>, name: &str, value: Option<bool>) {
    if let Some(value) = value {
        let text = if value { "true" } else { "false" };
        headers.insert(name.to_owned(), text.to_owned());
    }
}

pub fn prepare_headers(
    settings: &PillarSettings,
    key: &KeyContext,
) -> Result<BTreeMap<String, String>, PillarError> {
    let Some(api_key) = settings
        .api_key
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
    else {
        return Err(PillarError::MissingApiKey);
    };
    let mut headers = BTreeMap::new();
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", api_key.expose_secret()),
    );
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    set_bool_header(
        &mut headers,
        "plr_scanners",
        Some(settings.include_scanners),
    );
    set_bool_header(
        &mut headers,
        "plr_evidence",
        Some(settings.include_evidence),
    );
    set_bool_header(&mut headers, "plr_async", settings.async_mode);
    set_bool_header(&mut headers, "plr_persist", settings.persist_session);
    let fields = [
        ("X-LiteLLM-Key-Name", &key.key_name),
        ("X-LiteLLM-Key-Alias", &key.key_alias),
        ("X-LiteLLM-User-Id", &key.user_id),
        ("X-LiteLLM-User-Email", &key.user_email),
        ("X-LiteLLM-Team-Id", &key.team_id),
        ("X-LiteLLM-Team-Name", &key.team_alias),
        ("X-LiteLLM-Org-Id", &key.org_id),
    ];
    for (name, value) in fields {
        let Some(value) = value.as_deref().filter(|value| !value.is_empty()) else {
            continue;
        };
        headers.insert(name.to_owned(), value.to_owned());
    }
    Ok(headers)
}
