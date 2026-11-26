use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use super::config::{LITELLM_SETTINGS, object_entries};
use super::error::SettingsUpdateError;
use super::keys::SettingKey;
use super::ports::{ConfigAudit, ConfigAuditEntry, LitellmSettingPublisher};
use super::records::SettingsRecord;
use super::response::{SettingsUpdateResponse, SettingsUpdateStatus};
use crate::audit_events::identity::CallerIdentity;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::proxy_config_state::database::ConfigStateDatabase;
use crate::proxy_config_state::save::save_config;
use crate::proxy_config_state::state::ProxyConfigState;

// The collaborators one update reaches: B8379's reload, the database behind B8380's save, the runtime litellm setting and B065's audit service
#[derive(Clone, Copy)]
pub struct SettingsUpdatePorts<'ports> {
    pub overlay: &'ports dyn ConfigOverlayPort,
    pub database: Option<&'ports dyn ConfigStateDatabase>,
    pub runtime: &'ports dyn LitellmSettingPublisher,
    pub audit: &'ports dyn ConfigAudit,
}

// One update call's typed arguments: the validated record, the litellm_settings key it replaces, the route's message and the acting admin
pub struct SettingsUpdate<'update> {
    pub record: &'update dyn SettingsRecord,
    pub setting: SettingKey,
    pub success_message: &'update str,
    pub admin: &'update CallerIdentity,
}

// _update_litellm_setting: publish the runtime value after the config read, then persist it and audit the successful save
pub async fn update_litellm_setting(
    ports: SettingsUpdatePorts<'_>,
    state: &mut ProxyConfigState,
    update: &SettingsUpdate<'_>,
) -> Result<SettingsUpdateResponse, SettingsUpdateError> {
    if !state.store_model_in_db_enabled() {
        return Err(SettingsUpdateError::StorageDisabled);
    }
    let values = pruned_values(update.record.values());
    let stored = JsonValue::Object(values.clone().into_iter().collect());
    let mut config = ports.overlay.reload_config().await?;
    // a null stored section reads as empty, but save_config diffs the baseline, which must be an object too
    let empty_section = JsonValue::Object(BTreeMap::new().into_iter().collect());
    if let Some(baseline) = config.baseline.as_mut()
        && baseline.get(LITELLM_SETTINGS) == Some(&JsonValue::Null)
    {
        baseline.insert(LITELLM_SETTINGS.to_owned(), empty_section);
    }
    let mut section = object_entries(config.values.get(LITELLM_SETTINGS), LITELLM_SETTINGS)?;
    let before_value = section.get(update.setting.as_str()).cloned();
    ports.runtime.publish(update.setting, &stored);
    section.insert(update.setting.as_str().to_owned(), stored.clone());
    let section_value = JsonValue::Object(section.into_iter().collect());
    config
        .values
        .insert(LITELLM_SETTINGS.to_owned(), section_value);
    save_config(state, &mut config, false, ports.database).await?;
    ports
        .audit
        .record_config_update(ConfigAuditEntry {
            setting: update.setting,
            before_value,
            after_value: stored,
            admin: update.admin.clone(),
        })
        .await;
    Ok(SettingsUpdateResponse {
        message: update.success_message.to_owned(),
        status: SettingsUpdateStatus::Success,
        settings: values,
    })
}

// model_dump(mode="json", exclude_none=True) drops every null object key, and a null list element keeps its position
fn pruned_values(values: BTreeMap<String, JsonValue>) -> BTreeMap<String, JsonValue> {
    values
        .into_iter()
        .filter_map(|(key, value)| {
            let pruned = prune_null_value(value);
            (!pruned.is_null()).then_some((key, pruned))
        })
        .collect()
}

fn prune_null_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => {
            let entries = object.into_iter().collect();
            JsonValue::Object(pruned_values(entries).into_iter().collect())
        }
        JsonValue::Array(items) => {
            JsonValue::Array(items.into_iter().map(prune_null_value).collect())
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => value,
    }
}
