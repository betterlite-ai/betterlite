use std::cmp::Ordering;
use std::collections::BTreeSet;

use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::sections::team_admin_fields::EDITABLE_FIELDS_SETTING;
use betterlite_config::ui_settings::{
    ALLOWED_UI_SETTINGS_FIELDS, CLIENT_HEADERS_SETTING, DISABLE_AGENTS_SETTING,
    DISABLE_CUSTOM_KEYS_SETTING, DISABLE_MODEL_ADD_SETTING, DISABLE_ORG_KEY_GENERATE,
    DISABLE_TEAM_DELETE_SETTING, DISABLE_VECTOR_STORES_SETTING, ENABLE_CHAT_UI_SETTING,
    HEALTH_DETAILS_SETTING, PAGES_SETTING, PROVIDER_HEADERS_SETTING, PUBLIC_AI_HUB_AUTH_SETTING,
    TEAM_ADMIN_AGENTS_SETTING, TEAM_ADMIN_VECTOR_STORES, USER_SEARCH_SCOPE_SETTING, UiSettingsDict,
};
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

use crate::scim_records::lax_bool::ScimBool;

// A bool on the wire; the newtype keeps struct_excessive_bools off a record that declares fourteen switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSettingFlag(pub bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSettingErrorKind {
    BoolParsing,
    BoolType,
    ListType,
    StringType,
    IsInstanceOf,
    SequenceStr,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind:?} for {field}")]
pub struct UiSettingsError {
    pub kind: UiSettingErrorKind,
    pub field: &'static str,
    pub index: Option<usize>,
}

// LiteLLM's UISettings model: fourteen booleans, the nullable page list, the team field list and any extra keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    disable_model_add: UiSettingFlag,
    disable_team_delete: UiSettingFlag,
    require_hub_auth: UiSettingFlag,
    health_details: UiSettingFlag,
    client_headers: UiSettingFlag,
    provider_headers: UiSettingFlag,
    disable_agents: UiSettingFlag,
    team_admin_agents: UiSettingFlag,
    disable_vector_stores: UiSettingFlag,
    team_admin_vector_stores: UiSettingFlag,
    user_search_scope: UiSettingFlag,
    disable_custom_keys: UiSettingFlag,
    org_key_generate: UiSettingFlag,
    chat_ui: UiSettingFlag,
    pages: Option<Vec<String>>,
    team_fields: Vec<String>,
    extras: UiSettingsDict,
    provided: BTreeSet<String>,
}

impl UiSettings {
    pub fn validate(input: &UiSettingsDict) -> Result<Self, UiSettingsError> {
        let extras: UiSettingsDict = input
            .iter()
            .filter(|(name, _)| !ALLOWED_UI_SETTINGS_FIELDS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let provided: BTreeSet<String> = input
            .keys()
            .filter(|name| ALLOWED_UI_SETTINGS_FIELDS.contains(&name.as_str()))
            .cloned()
            .collect();
        // Field order matches the pydantic model, so the first rejected field is the same one.
        Ok(Self {
            disable_model_add: setting_flag(input, DISABLE_MODEL_ADD_SETTING)?,
            disable_team_delete: setting_flag(input, DISABLE_TEAM_DELETE_SETTING)?,
            pages: pages_setting(input)?,
            require_hub_auth: setting_flag(input, PUBLIC_AI_HUB_AUTH_SETTING)?,
            health_details: setting_flag(input, HEALTH_DETAILS_SETTING)?,
            client_headers: setting_flag(input, CLIENT_HEADERS_SETTING)?,
            provider_headers: setting_flag(input, PROVIDER_HEADERS_SETTING)?,
            disable_agents: setting_flag(input, DISABLE_AGENTS_SETTING)?,
            team_admin_agents: setting_flag(input, TEAM_ADMIN_AGENTS_SETTING)?,
            disable_vector_stores: setting_flag(input, DISABLE_VECTOR_STORES_SETTING)?,
            team_admin_vector_stores: setting_flag(input, TEAM_ADMIN_VECTOR_STORES)?,
            user_search_scope: setting_flag(input, USER_SEARCH_SCOPE_SETTING)?,
            disable_custom_keys: setting_flag(input, DISABLE_CUSTOM_KEYS_SETTING)?,
            org_key_generate: setting_flag(input, DISABLE_ORG_KEY_GENERATE)?,
            chat_ui: setting_flag(input, ENABLE_CHAT_UI_SETTING)?,
            team_fields: team_fields_setting(input)?,
            extras,
            provided,
        })
    }

    #[must_use]
    pub fn model_dump(&self) -> UiSettingsDict {
        let mut values = self.known_dump();
        values.extend(self.extras.clone());
        values
    }

    #[must_use]
    pub fn model_dump_exclude_unset(&self) -> UiSettingsDict {
        let mut values: UiSettingsDict = self
            .known_dump()
            .into_iter()
            .filter(|(name, _)| self.provided.contains(name))
            .collect();
        values.extend(self.extras.clone());
        values
    }

    fn known_dump(&self) -> UiSettingsDict {
        let mut values = UiSettingsDict::new();
        for (name, flag) in self.flags() {
            values.insert(name.to_owned(), JsonValue::Bool(flag.0));
        }
        values.insert(
            PAGES_SETTING.to_owned(),
            match &self.pages {
                Some(pages) => JsonValue::Array(
                    pages
                        .iter()
                        .map(|page| JsonValue::String(page.clone()))
                        .collect(),
                ),
                None => JsonValue::Null,
            },
        );
        values.insert(
            EDITABLE_FIELDS_SETTING.to_owned(),
            JsonValue::Array(
                self.team_fields
                    .iter()
                    .map(|field| JsonValue::String(field.clone()))
                    .collect(),
            ),
        );
        values
    }

    const fn flags(&self) -> [(&'static str, UiSettingFlag); 14] {
        [
            (DISABLE_MODEL_ADD_SETTING, self.disable_model_add),
            (DISABLE_TEAM_DELETE_SETTING, self.disable_team_delete),
            (PUBLIC_AI_HUB_AUTH_SETTING, self.require_hub_auth),
            (HEALTH_DETAILS_SETTING, self.health_details),
            (CLIENT_HEADERS_SETTING, self.client_headers),
            (PROVIDER_HEADERS_SETTING, self.provider_headers),
            (DISABLE_AGENTS_SETTING, self.disable_agents),
            (TEAM_ADMIN_AGENTS_SETTING, self.team_admin_agents),
            (DISABLE_VECTOR_STORES_SETTING, self.disable_vector_stores),
            (TEAM_ADMIN_VECTOR_STORES, self.team_admin_vector_stores),
            (USER_SEARCH_SCOPE_SETTING, self.user_search_scope),
            (DISABLE_CUSTOM_KEYS_SETTING, self.disable_custom_keys),
            (DISABLE_ORG_KEY_GENERATE, self.org_key_generate),
            (ENABLE_CHAT_UI_SETTING, self.chat_ui),
        ]
    }
}

fn setting_flag(
    input: &UiSettingsDict,
    name: &'static str,
) -> Result<UiSettingFlag, UiSettingsError> {
    match input.get(name) {
        None => Ok(UiSettingFlag(false)),
        Some(value) => boolean(name, value),
    }
}

fn boolean(name: &'static str, value: &JsonValue) -> Result<UiSettingFlag, UiSettingsError> {
    match value {
        JsonValue::Bool(flag) => Ok(UiSettingFlag(*flag)),
        JsonValue::Number(number) => number_bool(name, number),
        JsonValue::String(_) => match serde_json::from_value::<ScimBool>(value.clone()) {
            Ok(ScimBool(flag)) => Ok(UiSettingFlag(flag)),
            Err(_) => Err(UiSettingsError {
                kind: UiSettingErrorKind::BoolParsing,
                field: name,
                index: None,
            }),
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => Err(UiSettingsError {
            kind: UiSettingErrorKind::BoolType,
            field: name,
            index: None,
        }),
    }
}

fn number_bool(name: &'static str, number: &Number) -> Result<UiSettingFlag, UiSettingsError> {
    if let Some(value) = number.as_i64() {
        return match value {
            0 => Ok(UiSettingFlag(false)),
            1 => Ok(UiSettingFlag(true)),
            _ => Err(UiSettingsError {
                kind: UiSettingErrorKind::BoolParsing,
                field: name,
                index: None,
            }),
        };
    }
    if number.is_f64() {
        let value = number.as_f64().unwrap_or(f64::NAN);
        if value.partial_cmp(&0.0) == Some(Ordering::Equal) {
            return Ok(UiSettingFlag(false));
        }
        if value.partial_cmp(&1.0) == Some(Ordering::Equal) {
            return Ok(UiSettingFlag(true));
        }
        if value.fract().partial_cmp(&0.0) == Some(Ordering::Equal)
            && value > -9_223_372_036_854_775_808.0_f64
            && value < 9_223_372_036_854_775_808.0_f64
        {
            return Err(UiSettingsError {
                kind: UiSettingErrorKind::BoolParsing,
                field: name,
                index: None,
            });
        }
    }
    Err(UiSettingsError {
        kind: UiSettingErrorKind::BoolType,
        field: name,
        index: None,
    })
}

// Both list fields share the item loop; only the whole-value error kind differs per field.
fn list_setting(
    value: &JsonValue,
    name: &'static str,
    text_error: UiSettingErrorKind,
    other_error: UiSettingErrorKind,
) -> Result<Vec<String>, UiSettingsError> {
    let Some(items) = value.as_array() else {
        let kind = if value.is_string() {
            text_error
        } else {
            other_error
        };
        return Err(UiSettingsError {
            kind,
            field: name,
            index: None,
        });
    };
    let mut strings = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let JsonValue::String(text) = item else {
            return Err(UiSettingsError {
                kind: UiSettingErrorKind::StringType,
                field: name,
                index: Some(index),
            });
        };
        strings.push(text.clone());
    }
    Ok(strings)
}

fn pages_setting(input: &UiSettingsDict) -> Result<Option<Vec<String>>, UiSettingsError> {
    match input.get(PAGES_SETTING) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => list_setting(
            value,
            PAGES_SETTING,
            UiSettingErrorKind::ListType,
            UiSettingErrorKind::ListType,
        )
        .map(Some),
    }
}

fn team_fields_setting(input: &UiSettingsDict) -> Result<Vec<String>, UiSettingsError> {
    let Some(value) = input.get(EDITABLE_FIELDS_SETTING) else {
        return Ok(Vec::new());
    };
    list_setting(
        value,
        EDITABLE_FIELDS_SETTING,
        UiSettingErrorKind::SequenceStr,
        UiSettingErrorKind::IsInstanceOf,
    )
}
