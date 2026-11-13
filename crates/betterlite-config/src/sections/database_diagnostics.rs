use betterlite_db::database_diagnostics::initialization::DiagnosticSettings;
use betterlite_db::database_diagnostics::json_enabled::environment_enabled;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

impl Section for Raw {
    type Settings = DiagnosticSettings;

    fn convert<L>(self, lookup: &L) -> Result<DiagnosticSettings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let level = lookup
            .lookup("LITELLM_LOG")
            .unwrap_or_else(|| "INFO".to_owned());
        let json = environment_enabled(lookup.lookup("JSON_LOGS").as_deref());
        Ok(DiagnosticSettings { level, json })
    }
}
