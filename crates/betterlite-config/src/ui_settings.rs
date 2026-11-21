use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, parse_view};
use crate::sections::settings::settings_rules::JsonValue;
use crate::sections::settings::settings_store::SettingsStore;
use crate::sections::team_admin_fields::EDITABLE_FIELDS_SETTING;

mod tests;

pub const DISABLE_AGENTS_SETTING: &str = "disable_agents_for_internal_users";
pub const TEAM_ADMIN_AGENTS_SETTING: &str = "allow_agents_for_team_admins";
pub const PUBLIC_AI_HUB_AUTH_SETTING: &str = "require_auth_for_public_ai_hub";
pub const HEALTH_DETAILS_SETTING: &str = "allow_public_health_readiness_details";
pub const CLIENT_HEADERS_SETTING: &str = "forward_client_headers_to_llm_api";
pub const PROVIDER_HEADERS_SETTING: &str = "forward_llm_provider_auth_headers";
pub const DISABLE_VECTOR_STORES_SETTING: &str = "disable_vector_stores_for_internal_users";
pub const TEAM_ADMIN_VECTOR_STORES: &str = "allow_vector_stores_for_team_admins";
pub const DISABLE_CUSTOM_KEYS_SETTING: &str = "disable_custom_api_keys";
pub const DISABLE_ORG_KEY_GENERATE: &str = "disable_key_generate_for_org_admin";
pub const ENABLE_CHAT_UI_SETTING: &str = "enable_chat_ui";
pub const PAGES_SETTING: &str = "enabled_ui_pages_internal_users";
pub const USER_SEARCH_SCOPE_SETTING: &str = "scope_user_search_to_org";
pub const DISABLE_MODEL_ADD_SETTING: &str = "disable_model_add_for_internal_users";
pub const DISABLE_TEAM_DELETE_SETTING: &str = "disable_team_admin_delete_team_user";

pub const PTU_COST_ATTRIBUTION_SETTING: &str = "enable_ptu_cost_attribution";
pub const USER_BUDGET_TEAM_KEYS_SETTING: &str = "apply_user_budget_to_team_keys";

pub type UiSettingsDict = BTreeMap<String, JsonValue>;

pub const ALLOWED_UI_SETTINGS_FIELDS: [&str; 16] = [
    TEAM_ADMIN_AGENTS_SETTING,
    HEALTH_DETAILS_SETTING,
    TEAM_ADMIN_VECTOR_STORES,
    DISABLE_AGENTS_SETTING,
    DISABLE_CUSTOM_KEYS_SETTING,
    DISABLE_ORG_KEY_GENERATE,
    DISABLE_MODEL_ADD_SETTING,
    DISABLE_TEAM_DELETE_SETTING,
    DISABLE_VECTOR_STORES_SETTING,
    ENABLE_CHAT_UI_SETTING,
    PAGES_SETTING,
    CLIENT_HEADERS_SETTING,
    PROVIDER_HEADERS_SETTING,
    PUBLIC_AI_HUB_AUTH_SETTING,
    USER_SEARCH_SCOPE_SETTING,
    EDITABLE_FIELDS_SETTING,
];

pub const BOOLEAN_UI_SETTINGS_FIELDS: [&str; 14] = [
    TEAM_ADMIN_AGENTS_SETTING,
    HEALTH_DETAILS_SETTING,
    TEAM_ADMIN_VECTOR_STORES,
    DISABLE_AGENTS_SETTING,
    DISABLE_CUSTOM_KEYS_SETTING,
    DISABLE_ORG_KEY_GENERATE,
    DISABLE_MODEL_ADD_SETTING,
    DISABLE_TEAM_DELETE_SETTING,
    DISABLE_VECTOR_STORES_SETTING,
    ENABLE_CHAT_UI_SETTING,
    CLIENT_HEADERS_SETTING,
    PROVIDER_HEADERS_SETTING,
    PUBLIC_AI_HUB_AUTH_SETTING,
    USER_SEARCH_SCOPE_SETTING,
];

// The pydantic defaults of the UISettings fields, which source reporting falls back to.
#[must_use]
pub fn field_default(key: &str) -> JsonValue {
    if BOOLEAN_UI_SETTINGS_FIELDS.contains(&key) {
        JsonValue::Bool(false)
    } else if key == PAGES_SETTING {
        JsonValue::Null
    } else if key == EDITABLE_FIELDS_SETTING {
        JsonValue::Array(Vec::new())
    } else {
        JsonValue::Null
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    general_settings: Option<BTreeMap<String, JsonValue>>,
}

// ProxyConfig keeps every general_settings key; the UI model reads the sixteen it knows.
pub fn load_ui_settings(source: &str) -> Result<UiSettingsDict, ConfigError> {
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    Ok(raw
        .general_settings
        .unwrap_or_default()
        .into_iter()
        .filter(|(name, _)| ALLOWED_UI_SETTINGS_FIELDS.contains(&name.as_str()))
        .collect())
}

// The config values the UI settings endpoint overlays on the stored row, as proxy_config.settings does.
#[must_use]
pub fn ui_settings_from_store(store: &SettingsStore) -> UiSettingsDict {
    ALLOWED_UI_SETTINGS_FIELDS
        .iter()
        .filter_map(|name| {
            store
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect()
}
