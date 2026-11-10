use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use secrecy::ExposeSecret as _;

use super::choices::{ModelChoice, StaticToken};
use super::statusline_install::is_our_statusline;
use super::{
    ANTHROPIC_API_KEY_KEY, ANTHROPIC_AUTH_TOKEN_KEY, ANTHROPIC_BASE_URL_KEY, ANTHROPIC_MODEL_KEY,
    API_KEY_HELPER_KEY, ENABLE_TOOL_SEARCH_KEY, ENABLE_TOOL_SEARCH_VALUE, ENV_KEY,
    GATEWAY_DISCOVERY_KEY, GATEWAY_DISCOVERY_VALUE, MODEL_KEY, STATUS_LINE_KEY, SettingsDocument,
    TIER_MODEL_ENV_KEYS,
};

#[derive(Debug)]
pub struct MergeOptions<'input> {
    pub base_url: &'input str,
    pub credential: &'input StaticToken,
    pub model: &'input ModelChoice,
    pub tier_model: Option<&'input str>,
    pub status_line: Option<&'input str>,
}

#[must_use]
pub fn merge_claude_settings(
    settings: &SettingsDocument,
    options: &MergeOptions<'_>,
) -> SettingsDocument {
    let base = options.status_line.map_or_else(
        || settings.clone(),
        |command| with_status_line(settings, command),
    );
    let mut document = SettingsDocument::new();
    for (key, value) in &base {
        if key != ENV_KEY && key != API_KEY_HELPER_KEY {
            document.insert(key.clone(), value.clone());
        }
    }
    document.insert(ENV_KEY.to_owned(), merged_env(settings, options));
    if let Some(model) = options.model.starting_model() {
        document.insert(MODEL_KEY.to_owned(), JsonValue::String(model.to_owned()));
    }
    document
}

#[must_use]
pub fn with_status_line(settings: &SettingsDocument, command: &str) -> SettingsDocument {
    let ours = match settings.get(STATUS_LINE_KEY) {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::Object(entry)) => entry
            .get("command")
            .and_then(JsonValue::as_str)
            .is_some_and(|existing| is_our_statusline(existing, command)),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => false,
    };
    let mut updated = settings.clone();
    if !ours {
        return updated;
    }
    let mut entry = SettingsDocument::new();
    entry.insert("type".to_owned(), JsonValue::String("command".to_owned()));
    entry.insert("command".to_owned(), JsonValue::String(command.to_owned()));
    updated.insert(
        STATUS_LINE_KEY.to_owned(),
        JsonValue::Object(entry.into_iter().collect()),
    );
    updated
}

fn merged_env(settings: &SettingsDocument, options: &MergeOptions<'_>) -> JsonValue {
    let mut env = BTreeMap::new();
    env.insert(
        ENABLE_TOOL_SEARCH_KEY.to_owned(),
        JsonValue::String(ENABLE_TOOL_SEARCH_VALUE.to_owned()),
    );
    env.insert(
        GATEWAY_DISCOVERY_KEY.to_owned(),
        JsonValue::String(GATEWAY_DISCOVERY_VALUE.to_owned()),
    );
    if let Some(JsonValue::Object(current)) = settings.get(ENV_KEY) {
        for (key, value) in current {
            if key != ANTHROPIC_API_KEY_KEY && key != ANTHROPIC_AUTH_TOKEN_KEY {
                env.insert(key.clone(), value.clone());
            }
        }
    }
    env.insert(
        ANTHROPIC_BASE_URL_KEY.to_owned(),
        JsonValue::String(options.base_url.trim_end_matches('/').to_owned()),
    );
    let token = JsonValue::String(options.credential.token.expose_secret().to_owned());
    env.insert(ANTHROPIC_AUTH_TOKEN_KEY.to_owned(), token);
    if let Some(model) = options.model.starting_model() {
        env.insert(
            ANTHROPIC_MODEL_KEY.to_owned(),
            JsonValue::String(model.to_owned()),
        );
    }
    if let Some(tier_model) = options.tier_model {
        for key in TIER_MODEL_ENV_KEYS {
            env.insert(key.to_owned(), JsonValue::String(tier_model.to_owned()));
        }
    }
    JsonValue::Object(env.into_iter().collect())
}

mod tests;
