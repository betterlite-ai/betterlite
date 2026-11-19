use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::audit_events::Raw as SectionRaw;
use crate::sections::audit_events::Settings as SectionSettings;
use crate::sections::settings::JsonValue;

mod tests;

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub config: SectionSettings,
    pub audit_log_callbacks_present: bool,
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
    let (raw, _unknown_section) = parse_view::<SectionRaw>(source)?;
    let config = Section::convert(raw, lookup)?;
    let (presence, _unknown_presence) = parse_view::<Presence>(source)?;
    Ok(Settings {
        config,
        audit_log_callbacks_present: presence.litellm_settings.audit_log_callbacks.is_some(),
    })
}
