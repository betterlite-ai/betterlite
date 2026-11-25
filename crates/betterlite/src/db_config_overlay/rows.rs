use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::settings_store::config_ownership_message;
use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_server::db_row_decrypt::env::decrypt_and_export_env;

use super::error::OverlayError;
use super::scrub::scrub_remote_modules;
use super::state::{
    OverlayDatabase, OverlayState, RuntimeEffect, STORE_SECTIONS, settings_mapping, store_mut,
};

// LITELLM_SETTINGS_SAFE_DB_OVERRIDES: the litellm_settings keys a stored row also sets on the running proxy
const SAFE_DB_OVERRIDES: [&str; 23] = [
    "default_internal_user_params",
    "default_team_params",
    "public_mcp_servers",
    "public_agent_groups",
    "public_model_groups",
    "public_model_groups_links",
    "cost_discount_config",
    "cost_margin_config",
    "block_requests_for_models_without_pricing",
    "budget_exceeded_throttle_percentage",
    "log_auth_failure_key_identity",
    "enable_anthropic_prompt_caching",
    "anthropic_prompt_caching_ttl",
    "openai_system_messages_first",
    "max_ui_session_budget",
    "budget_rollover",
    "mcp_tool_search",
    "turn_off_message_logging",
    "datadog_params",
    "datadog_llm_observability_params",
    "newrelic_params",
    "pointfive_params",
    "aws_sqs_callback_params",
];

// LiteLLM sorts the shadowed (section, key) pairs, and these are the four sections in name order
const WARNING_ORDER: [Section; 4] = [
    Section::EnvironmentVariables,
    Section::GeneralSettings,
    Section::LitellmSettings,
    Section::RouterSettings,
];

// get_config_param: all four rows are read at once, before the caller takes its state lock
pub async fn fetch_db_rows(
    database: &OverlayDatabase,
) -> Result<[Option<JsonValue>; 4], OverlayError> {
    let [general, router, litellm, environment] = STORE_SECTIONS.map(Section::name);
    let fetched = tokio::try_join!(
        database.rows.config_param(general),
        database.rows.config_param(router),
        database.rows.config_param(litellm),
        database.rows.config_param(environment),
    )
    .map_err(|source| OverlayError::Database { source })?;
    Ok(<[Option<JsonValue>; 4]>::from(fetched))
}

// _update_config_from_db's second half: each present row is prepared and applied to its store
pub fn apply_db_rows(
    state: &mut OverlayState,
    stored: [Option<JsonValue>; 4],
    database: &OverlayDatabase,
    env: &mut BTreeMap<String, String>,
) {
    for (section, row) in STORE_SECTIONS.into_iter().zip(stored) {
        let Some(value) = row.filter(|value| !value.is_null()) else {
            continue;
        };
        let prepared = prepare_row(section, &value, database, env);
        if section == Section::LitellmSettings {
            apply_litellm_settings(state, prepared);
            continue;
        }
        if let Some(store) = store_mut(&mut state.config, section) {
            store.apply_db_row(DbRow::from(section), prepared);
        }
    }
    warn_shadowed_keys(state);
}

// _prepared_db_settings_values: environment values are decrypted and exported under both names, the rest scrubbed
fn prepare_row(
    section: Section,
    value: &JsonValue,
    database: &OverlayDatabase,
    env: &mut BTreeMap<String, String>,
) -> BTreeMap<String, JsonValue> {
    if section != Section::EnvironmentVariables {
        return settings_mapping(Some(&scrub_remote_modules(section, value)));
    }
    let Some(values) = value.as_object() else {
        return BTreeMap::new();
    };
    let exported = decrypt_and_export_env(values, true, &database.cipher, env, &database.warnings);
    let mut prepared = BTreeMap::new();
    for (key, text) in exported {
        let upper = key.to_uppercase();
        env.insert(upper.clone(), text.clone());
        prepared.insert(upper, JsonValue::String(text.clone()));
        prepared.insert(key, JsonValue::String(text));
    }
    prepared
}

// _apply_litellm_settings_db_values: a safe key the row sets reaches the running proxy with the store's value
pub fn apply_litellm_settings(state: &mut OverlayState, values: BTreeMap<String, JsonValue>) {
    let overridden: Vec<&'static str> = SAFE_DB_OVERRIDES
        .into_iter()
        .filter(|key| values.contains_key(*key))
        .collect();
    let store = &mut state.config.litellm_settings;
    store.apply_db_row(DbRow::LitellmSettings, values);
    for key in overridden {
        if let Some(value) = store.get(key).filter(|value| !value.is_null()) {
            state.runtime.apply(RuntimeEffect::LitellmSetting {
                key,
                value: value.clone(),
            });
        }
    }
}

// _warn_about_shadowed_db_settings: a key warns when it starts being shadowed, and again only after it stopped
fn warn_shadowed_keys(state: &mut OverlayState) {
    let mut warned = BTreeSet::new();
    for section in WARNING_ORDER {
        let store = state.config.settings_store(section);
        let keys = store
            .map(SettingsStore::shadowed_db_keys)
            .unwrap_or_default();
        for key in keys {
            let shadowed = (section.name(), key.clone());
            if !state.warned_shadowed.contains(&shadowed) {
                tracing::warn!("{}", config_ownership_message(section, &key, true));
            }
            warned.insert(shadowed);
        }
    }
    state.warned_shadowed = warned;
}
