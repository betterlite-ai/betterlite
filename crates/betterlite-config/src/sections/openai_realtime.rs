use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const MAX_MESSAGE_SIZE_VARIABLE: &str = "REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES";
pub const DEFAULT_MAX_MESSAGE_SIZE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub max_message_size_bytes: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_message_size_bytes: DEFAULT_MAX_MESSAGE_SIZE_BYTES,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // An unset variable reads as empty text, and the shared parser treats empty as unset.
        let text = lookup.lookup(MAX_MESSAGE_SIZE_VARIABLE).unwrap_or_default();
        let max_message_size_bytes =
            betterlite_env::number::<usize>(MAX_MESSAGE_SIZE_VARIABLE, &text)?
                .unwrap_or(DEFAULT_MAX_MESSAGE_SIZE_BYTES);
        Ok(Settings {
            max_message_size_bytes,
        })
    }
}
