use std::collections::BTreeMap;

use betterlite_config::sections::settings::{DbRow, JsonValue, Section};

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::sink::AuditEventSink;
use crate::config_audit::{ConfigChange, config_audit_log};

use super::error::ConfigFieldError;
use super::litellm_fields::{UI_FIELDS, UiLitellmField};
use super::plugin_keys::preserve_redacted_plugin_keys;
use super::ports::{
    AdminGate, ConfigFieldCaller, ConfigFieldDatabase, ConfigFieldServices, object_entries,
    require_admin, require_database,
};
use super::records::{
    ConfigFieldDelete, ConfigFieldUpdate, ConfigFieldUpdateResponse, GENERAL_SETTINGS,
    GeneralSettingsRow,
};
use super::ui::{persist_ui_litellm_field, reset_ui_litellm_field};
use super::validation::{kind_of, validate, validate_alerting_args};

const ALERTING_ARGS: &str = "alerting_args";
const PLUGINS: &str = "plugins";

// POST /config/field/update
pub async fn update_config_general_settings(
    services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
    update: &ConfigFieldUpdate,
) -> Result<ConfigFieldUpdateResponse, ConfigFieldError> {
    let database = require_database(services.database)?;
    require_admin(caller, AdminGate::Proxy)?;
    let field_name = update.field_name.as_str();
    if let Some(field) = ui_field(field_name) {
        let body = persist_ui_litellm_field(services, caller, field, &update.field_value).await?;
        return Ok(ConfigFieldUpdateResponse::Ui(body));
    }
    let Some(kind) = kind_of(field_name) else {
        return Err(invalid_field(field_name));
    };
    validate(kind, &update.field_value)?;
    if field_name == ALERTING_ARGS {
        validate_alerting_args(&update.field_value)?;
    }
    let row = write_field(services, database, caller, field_name, &update.field_value).await?;
    Ok(ConfigFieldUpdateResponse::Row(row))
}

// POST /config/field/delete
pub async fn delete_config_general_settings(
    services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
    delete: &ConfigFieldDelete,
) -> Result<ConfigFieldUpdateResponse, ConfigFieldError> {
    let database = require_database(services.database)?;
    require_admin(caller, AdminGate::ProxyWithRole)?;
    let field_name = delete.field_name.as_str();
    if let Some(field) = ui_field(field_name) {
        let body = reset_ui_litellm_field(services, caller, field).await?;
        return Ok(ConfigFieldUpdateResponse::Ui(body));
    }
    let Some(_kind) = kind_of(field_name) else {
        return Err(invalid_field(field_name));
    };
    let row = delete_field(services, database, caller, field_name).await?;
    Ok(ConfigFieldUpdateResponse::Row(row))
}

fn ui_field(name: &str) -> Option<UiLitellmField> {
    for field in UI_FIELDS {
        if field.name == name {
            return Some(field);
        }
    }
    None
}

fn invalid_field(name: &str) -> ConfigFieldError {
    ConfigFieldError::BadRequest {
        detail: format!("Invalid field={name} passed in."),
    }
}

// the row edit, upsert and in-memory apply the two routes share
async fn store_row(
    services: &mut ConfigFieldServices<'_>,
    database: &ConfigFieldDatabase,
    row_value: &JsonValue,
    entries: &BTreeMap<String, JsonValue>,
) -> Result<(), ConfigFieldError> {
    database
        .store
        .insert_config(&[(GENERAL_SETTINGS.to_owned(), row_value.clone())])
        .await?;
    services.state.apply_section_row(
        Section::GeneralSettings,
        DbRow::GeneralSettings,
        entries.clone(),
    );
    Ok(())
}

// the audit row both routes write, redacted and never failing the write
async fn record_change(
    audit: &AuditEventSink,
    caller: &ConfigFieldCaller,
    action: AuditAction,
    before: &JsonValue,
    after: &JsonValue,
) {
    let identity = caller.audit_identity();
    config_audit_log(
        audit,
        ConfigChange {
            param: GENERAL_SETTINGS,
            action,
            table: AuditTable::Config,
            before: Some(before),
            after: Some(after),
            caller: &identity,
        },
    )
    .await;
}

async fn write_field(
    mut services: ConfigFieldServices<'_>,
    database: &ConfigFieldDatabase,
    caller: &ConfigFieldCaller,
    field_name: &str,
    value: &JsonValue,
) -> Result<GeneralSettingsRow, ConfigFieldError> {
    let existing = database.rows.config_param(GENERAL_SETTINGS).await?;
    let mut entries = object_entries(existing.as_ref())?;
    let before_value = JsonValue::Object(entries.clone().into_iter().collect());
    let field_value = if field_name == PLUGINS {
        preserve_redacted_plugin_keys(value, entries.get(PLUGINS))
    } else {
        value.clone()
    };
    entries.insert(field_name.to_owned(), field_value);
    let mut changed = BTreeMap::new();
    changed.insert(field_name.to_owned(), value.clone());
    services
        .state
        .reject_config_owned_writes(Section::GeneralSettings, &changed)?;
    let row_value = JsonValue::Object(entries.clone().into_iter().collect());
    store_row(&mut services, database, &row_value, &entries).await?;
    serve_pass_through(&services, field_name, &entries).await?;
    record_change(
        services.audit,
        caller,
        AuditAction::Updated,
        &before_value,
        &row_value,
    )
    .await;
    if field_name == PLUGINS {
        let settings = row_value.as_object().ok_or(ConfigFieldError::Internal)?;
        services.plugins.replace_from_settings(settings)?;
    }
    services.runtime.apply_ssrf_settings(&entries);
    Ok(GeneralSettingsRow {
        param_name: GENERAL_SETTINGS,
        param_value: row_value,
    })
}

async fn delete_field(
    mut services: ConfigFieldServices<'_>,
    database: &ConfigFieldDatabase,
    caller: &ConfigFieldCaller,
    field_name: &str,
) -> Result<GeneralSettingsRow, ConfigFieldError> {
    services
        .state
        .reject_config_owned_deletes(Section::GeneralSettings, &[field_name.to_owned()])?;
    let existing = database.rows.config_param(GENERAL_SETTINGS).await?;
    let Some(stored) = existing.filter(|value| !value.is_null()) else {
        return Err(ConfigFieldError::BadRequest {
            detail: format!("Field name={field_name} not in config"),
        });
    };
    let mut entries = object_entries(Some(&stored))?;
    let before_value = JsonValue::Object(entries.clone().into_iter().collect());
    entries.remove(field_name);
    let row_value = JsonValue::Object(entries.clone().into_iter().collect());
    store_row(&mut services, database, &row_value, &entries).await?;
    serve_pass_through(&services, field_name, &entries).await?;
    record_change(
        services.audit,
        caller,
        AuditAction::Deleted,
        &before_value,
        &row_value,
    )
    .await;
    Ok(GeneralSettingsRow {
        param_name: GENERAL_SETTINGS,
        param_value: row_value,
    })
}

// a resource-list write re-serves the pass-through router with the row's list
async fn serve_pass_through(
    services: &ConfigFieldServices<'_>,
    field_name: &str,
    entries: &BTreeMap<String, JsonValue>,
) -> Result<(), ConfigFieldError> {
    if !Section::GeneralSettings.is_resource_list(field_name) {
        return Ok(());
    }
    services
        .overlay
        .serve_pass_through_endpoints(&pass_through_list(entries))
        .await?;
    Ok(())
}

// the written row's list, or an empty one when the field is missing or is not a list
fn pass_through_list(entries: &BTreeMap<String, JsonValue>) -> Vec<JsonValue> {
    match entries.get("pass_through_endpoints") {
        Some(JsonValue::Array(items)) => items.clone(),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Object(_),
        )
        | None => Vec::new(),
    }
}
