use std::collections::BTreeMap;

use betterlite_config::sections::settings::settings_store::Resolution;
use betterlite_config::sections::settings::{FieldSource, JsonValue, Section, SettingsStore};
use betterlite_redact::setting_redaction::redact_setting_value;

use crate::proxy_config_state::state::ProxyConfigState;

use super::error::ConfigFieldError;
use super::fields::{
    LIST_DESCRIPTIONS, LIST_FIELDS, ListDefault, NESTED_DESCRIPTIONS, NESTED_FIELDS,
};
use super::litellm_fields::{UI_FIELDS, UiLitellmField, default_value};
use super::plugin_keys::PLUGIN_KEY_REDACTED;
use super::ports::{
    AdminGate, ConfigFieldCaller, ConfigFieldDatabase, ConfigFieldRuntime, ConfigFieldServices,
    PROXY_ADMIN_ROLE, object_entries, require_admin, require_database,
};
use super::records::{ConfigFieldInfo, ConfigList, FieldDetail, GENERAL_SETTINGS};
use super::validation::kind_of;

const PASSTHROUGH: &str = "pass_through_endpoints";
const LITELLM_SETTINGS: &str = "litellm_settings";
const PLUGINS: &str = "plugins";

// GET /config/field/info
pub async fn get_config_general_settings(
    services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
    field_name: &str,
) -> Result<ConfigFieldInfo, ConfigFieldError> {
    let database = require_database(services.database)?;
    require_admin(caller, AdminGate::View)?;
    if kind_of(field_name).is_none() {
        return Err(ConfigFieldError::BadRequest {
            detail: format!("Invalid field={field_name} passed in."),
        });
    }
    let Some(value) = declared_value(services.state, database, field_name).await? else {
        return Err(ConfigFieldError::BadRequest {
            detail: format!("Field name={field_name} is not set"),
        });
    };
    let full_admin = caller.user_role.as_deref() == Some(PROXY_ADMIN_ROLE);
    let redacted = redact_setting_value(field_name, &value, full_admin);
    let field_value = if field_name == PLUGINS {
        hide_plugin_keys(&redacted)
    } else {
        redacted
    };
    let source = services
        .state
        .general_settings
        .resolution(field_name)
        .source();
    Ok(ConfigFieldInfo {
        field_name: field_name.to_owned(),
        field_value,
        source,
        editable: source != FieldSource::Config,
    })
}

// _declared_general_setting: resource lists come from the writer row, config-owned keys from the file, the rest resolved
async fn declared_value(
    state: &ProxyConfigState,
    database: &ConfigFieldDatabase,
    field_name: &str,
) -> Result<Option<JsonValue>, ConfigFieldError> {
    if Section::GeneralSettings.is_resource_list(field_name) {
        let row = database.rows.config_param(GENERAL_SETTINGS).await?;
        let stored = row.as_ref().and_then(JsonValue::as_object);
        return Ok(stored
            .and_then(|entries| entries.get(field_name))
            .filter(|value| !value.is_null())
            .cloned());
    }
    let store = &state.general_settings;
    Ok(match store.resolution(field_name) {
        Resolution::Config(value) => Some(value.clone()),
        Resolution::Db(_) | Resolution::Unset => store.get(field_name).cloned(),
    })
}

// /config/field/info redacts every plugin_key to "***" even for a full admin
fn hide_plugin_keys(value: &JsonValue) -> JsonValue {
    let Some(items) = value.as_array() else {
        return value.clone();
    };
    JsonValue::Array(
        items
            .iter()
            .map(|item| match item {
                JsonValue::Object(entries) if entries.contains_key("plugin_key") => {
                    let mut copy = entries.clone();
                    copy.insert(
                        "plugin_key".to_owned(),
                        JsonValue::String(PLUGIN_KEY_REDACTED.to_owned()),
                    );
                    JsonValue::Object(copy)
                }
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => item.clone(),
            })
            .collect(),
    )
}

// GET /config/list
pub async fn list_config_general_settings(
    services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
) -> Result<Vec<ConfigList>, ConfigFieldError> {
    let database = require_database(services.database)?;
    require_admin(caller, AdminGate::ViewWithRole)?;
    let context = ListContext {
        state: services.state,
        runtime: services.runtime,
        row: table_row(database, GENERAL_SETTINGS).await?,
        litellm: table_row(database, LITELLM_SETTINGS).await?,
        is_full_admin: caller.user_role.as_deref() == Some(PROXY_ADMIN_ROLE),
    };
    let mut entries = Vec::with_capacity(LIST_FIELDS.len() + UI_FIELDS.len());
    for ((name, field_type, default), description) in LIST_FIELDS.iter().zip(LIST_DESCRIPTIONS) {
        entries.push(list_entry(
            &context,
            *name,
            *field_type,
            *default,
            description,
        ));
    }
    for field in UI_FIELDS {
        entries.push(ui_entry(&context, field));
    }
    Ok(entries)
}

struct ListContext<'run> {
    state: &'run ProxyConfigState,
    runtime: &'run dyn ConfigFieldRuntime,
    row: BTreeMap<String, JsonValue>,
    litellm: BTreeMap<String, JsonValue>,
    is_full_admin: bool,
}

async fn table_row(
    database: &ConfigFieldDatabase,
    name: &'static str,
) -> Result<BTreeMap<String, JsonValue>, ConfigFieldError> {
    let row = database.rows.config_param(name).await?;
    object_entries(row.as_ref())
}

fn list_entry(
    context: &ListContext<'_>,
    name: &'static str,
    field_type: &'static str,
    default: ListDefault,
    description: &'static str,
) -> ConfigList {
    let store = &context.state.general_settings;
    let source = store.resolution(name).source();
    let value = list_value(store, &context.row, name, source);
    ConfigList {
        field_name: name.to_owned(),
        field_type,
        field_description: description,
        field_value: redact_setting_value(name, &value, context.is_full_admin),
        stored_in_db: stored_in_db(store, &context.row, name),
        field_default_value: default.value(),
        premium_field: false,
        nested_fields: (name == PASSTHROUGH).then(|| nested_fields(store, context.is_full_admin)),
        field_options: None,
        field_tab: None,
        source,
        editable: source != FieldSource::Config,
    }
}

// general_settings.get(field, None), falling back to the fresh row only when it is null and the file does not own it
fn list_value(
    store: &SettingsStore,
    row: &BTreeMap<String, JsonValue>,
    name: &str,
    source: FieldSource,
) -> JsonValue {
    let value = store.get(name).cloned().unwrap_or(JsonValue::Null);
    if !value.is_null() || source == FieldSource::Config || name == PASSTHROUGH {
        return value;
    }
    row.get(name).cloned().unwrap_or(JsonValue::Null)
}

fn stored_in_db(
    store: &SettingsStore,
    row: &BTreeMap<String, JsonValue>,
    name: &str,
) -> Option<bool> {
    if row.contains_key(name) {
        return Some(true);
    }
    store.get(name).is_some().then_some(false)
}

fn nested_fields(store: &SettingsStore, is_full_admin: bool) -> Vec<FieldDetail> {
    NESTED_FIELDS
        .iter()
        .zip(NESTED_DESCRIPTIONS)
        .map(|((name, field_type), description)| {
            let stored = store.get(*name).cloned().unwrap_or(JsonValue::Null);
            FieldDetail {
                field_name: *name,
                field_type: *field_type,
                field_description: description,
                field_default_value: redact_setting_value(*name, &stored, is_full_admin),
                stored_in_db: None,
            }
        })
        .collect()
}

fn ui_entry(context: &ListContext<'_>, field: UiLitellmField) -> ConfigList {
    let current = context.runtime.ui_litellm_field(field);
    let default = default_value(field);
    let source = context
        .state
        .litellm_settings
        .resolution(field.name)
        .source();
    ConfigList {
        field_name: field.name.to_owned(),
        field_type: field.kind.name(),
        field_description: field.description,
        field_value: current.clone(),
        stored_in_db: ui_stored_in_db(&context.litellm, field, &current, &default),
        field_default_value: default,
        premium_field: false,
        nested_fields: None,
        field_options: field.options.map(ToOwned::to_owned),
        field_tab: field.tab,
        source,
        editable: source != FieldSource::Config,
    }
}

fn ui_stored_in_db(
    litellm: &BTreeMap<String, JsonValue>,
    field: UiLitellmField,
    current: &JsonValue,
    default: &JsonValue,
) -> Option<bool> {
    if litellm.contains_key(field.name) {
        return Some(true);
    }
    (!same_value(current, default)).then_some(false)
}

// Python's ==, where 1 and 1.0 are the same number
fn same_value(left: &JsonValue, right: &JsonValue) -> bool {
    if left.is_number() {
        return left.as_f64() == right.as_f64();
    }
    left == right
}
