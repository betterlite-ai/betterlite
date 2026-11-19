use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::audit_events::Raw as SectionRaw;
use crate::sections::settings::JsonValue;

mod tests;

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub store_audit_logs: Option<bool>,
    pub store_audit_logs_env: Option<bool>,
    pub audit_log_callbacks: Vec<String>,
    pub audit_log_callbacks_present: bool,
    pub s3_audit_callback_params: Option<JsonValue>,
}

impl Settings {
    // is_audit_logging_enabled: an explicit argument, then the setting, then the environment, then on
    #[must_use]
    pub fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit
            .or(self.store_audit_logs)
            .or(self.store_audit_logs_env)
            .unwrap_or(true)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuditSettingsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
}

// the section's resolved settings drop whether the audit_log_callbacks key was written at all
#[derive(Debug, Default, Deserialize)]
struct Presence {
    #[serde(default)]
    litellm_settings: PresenceSettings,
}

#[derive(Debug, Default, Deserialize)]
struct PresenceSettings {
    #[serde(default)]
    audit_log_callbacks: Option<Vec<JsonValue>>,
}

pub fn load_audit_settings<L>(source: &str, lookup: &L) -> Result<Settings, AuditSettingsError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<SectionRaw>(source)?;
    let base = Section::convert(raw, lookup)?;
    let (presence, _ignored) = parse_view::<Presence>(source)?;
    Ok(Settings {
        store_audit_logs: base.store_audit_logs,
        store_audit_logs_env: base.store_audit_logs_env,
        audit_log_callbacks: base.audit_log_callbacks,
        audit_log_callbacks_present: presence.litellm_settings.audit_log_callbacks.is_some(),
        s3_audit_callback_params: base.s3_audit_callback_params,
    })
}

pub fn decode_audit_settings(source: &str) -> Result<Settings, AuditSettingsError> {
    load_audit_settings(source, &BTreeMap::<String, String>::new())
}
