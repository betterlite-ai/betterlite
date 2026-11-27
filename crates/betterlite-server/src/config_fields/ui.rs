use std::collections::BTreeMap;

use betterlite_config::sections::settings::{JsonValue, Section};

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::config_audit::{ConfigChange, config_audit_log};
use crate::proxy_config_state::save::save_config;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;

use super::error::ConfigFieldError;
use super::litellm_fields::{UiLitellmField, default_value, validate_ui_value};
use super::ports::{ConfigFieldCaller, ConfigFieldServices, require_database};
use super::records::UiFieldResponse;

const LITELLM_SETTINGS: &str = "litellm_settings";

// one admin UI edit: the value the caller sent, or the default a reset puts back
#[derive(Clone, Copy)]
pub enum UiFieldEdit<'value> {
    Persist(&'value JsonValue),
    Reset,
}

impl UiFieldEdit<'_> {
    const fn reset(self) -> bool {
        match self {
            Self::Persist(_) => false,
            Self::Reset => true,
        }
    }
}

// _persist_general_settings_ui_litellm_field and _reset_general_settings_ui_litellm_field: validate or default, then the shared save
pub async fn edit_ui_litellm_field(
    services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
    field: UiLitellmField,
    edit: UiFieldEdit<'_>,
) -> Result<UiFieldResponse, ConfigFieldError> {
    let validated = match edit {
        UiFieldEdit::Persist(value) => validate_ui_value(field, value)?,
        UiFieldEdit::Reset => default_value(field),
    };
    let message = write_ui_field(services, caller, field, validated, edit.reset()).await?;
    Ok(UiFieldResponse {
        message,
        status: "success",
    })
}

async fn write_ui_field(
    mut services: ConfigFieldServices<'_>,
    caller: &ConfigFieldCaller,
    field: UiLitellmField,
    validated: JsonValue,
    reset: bool,
) -> Result<String, ConfigFieldError> {
    let before = persist_section(&mut services, field, &validated, reset).await?;
    let identity = caller.audit_identity();
    config_audit_log(
        services.audit,
        ConfigChange {
            param: field.name,
            action: if reset {
                AuditAction::Deleted
            } else {
                AuditAction::Updated
            },
            table: AuditTable::Config,
            before: before.as_ref(),
            after: Some(&validated),
            caller: &identity,
        },
    )
    .await;
    let verb = if reset { "reset" } else { "updated" };
    Ok(format!("Field {} {verb}", field.name))
}

// the litellm_settings edit and the save behind it, answering the value the section held before
async fn persist_section(
    services: &mut ConfigFieldServices<'_>,
    field: UiLitellmField,
    validated: &JsonValue,
    reset: bool,
) -> Result<Option<JsonValue>, ConfigFieldError> {
    let database = require_database(services.database)?;
    let mut changed = BTreeMap::new();
    changed.insert(field.name.to_owned(), validated.clone());
    services
        .state
        .reject_config_owned_writes(Section::LitellmSettings, &changed)?;
    let mut config = services.overlay.reload_config().await?;
    let before = edit_section(&mut config, field, validated, reset)?;
    services
        .runtime
        .set_ui_litellm_field(field, validated.clone());
    save_config(
        services.state,
        &mut config,
        false,
        Some(database.store.as_ref()),
    )
    .await?;
    Ok(before)
}

// config["litellm_settings"][field] = value, or its pop for a reset; the before value is the reloaded section's
fn edit_section(
    config: &mut ConfigWithBaseline,
    field: UiLitellmField,
    validated: &JsonValue,
    reset: bool,
) -> Result<Option<JsonValue>, ConfigFieldError> {
    let mut section = match config.values.get(LITELLM_SETTINGS) {
        None | Some(JsonValue::Null) if reset => return Ok(None),
        None | Some(JsonValue::Null) => BTreeMap::new(),
        Some(JsonValue::Object(entries)) => entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => return Err(ConfigFieldError::Internal),
    };
    let before = section.get(field.name).cloned();
    if reset {
        section.remove(field.name);
    } else {
        section.insert(field.name.to_owned(), validated.clone());
    }
    config.values.insert(
        LITELLM_SETTINGS.to_owned(),
        JsonValue::Object(section.into_iter().collect()),
    );
    Ok(before)
}
