use serde::Deserialize;

use super::{SpendUpdates, spend_updates};
use crate::loader::{ConfigError, parse_view};
use crate::sections::settings::JsonValue;

mod tests;

// _ProxyDBLogger._should_track_errors_in_db reads general_settings.disable_error_logs
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    general_settings: GeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    disable_error_logs: JsonValue,
    disable_spend_updates: JsonValue,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub disable_error_logs: bool,
    // ProxyUpdateSpend.disable_spend_updates, the flag every spend writer and callback shares
    pub spend_updates: SpendUpdates,
}

pub fn loads_spend_failure_settings(source: &str) -> Result<Settings, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    let disable_error_logs = matches!(
        raw.general_settings.disable_error_logs,
        JsonValue::Bool(true)
    );
    Ok(Settings {
        disable_error_logs,
        spend_updates: spend_updates(&raw.general_settings.disable_spend_updates),
    })
}
