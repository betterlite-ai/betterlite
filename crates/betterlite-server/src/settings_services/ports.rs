use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;

use super::keys::SettingKey;
use crate::audit_events::identity::CallerIdentity;

// create_config_audit_log's arguments: the setting, the block it replaces, the block it stores and the acting admin
#[derive(Clone, Debug)]
pub struct ConfigAuditEntry {
    pub setting: SettingKey,
    pub before_value: Option<JsonValue>,
    pub after_value: JsonValue,
    pub admin: CallerIdentity,
}

// The config audit B065's bounded delivery service backs; B8612 implements it over B8594's config_audit_log with action "updated"
#[async_trait]
pub trait ConfigAudit: Send + Sync {
    async fn record_config_update(&self, entry: ConfigAuditEntry);
}

// LiteLLM's setattr(litellm, settings_key, value), typed to the five settings the update routes serve
pub trait LitellmSettingPublisher: Send + Sync {
    fn publish(&self, setting: SettingKey, value: &JsonValue);
}
