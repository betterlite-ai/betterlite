use std::path::PathBuf;

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};
use crate::sections::openai_realtime::Settings;

mod tests;

// B002's REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES read through the real loader, so the YAML and the environment meet where LiteLLM reads the constant
pub fn loads_realtime_backend_settings(
    source: &str,
    environment: &dyn EnvironmentLookup,
) -> Result<Settings, ConfigError> {
    let config = ConfigLoader {
        lookup: Borrowed(environment),
    }
    .load(source)?;
    Ok(config.sections.openai_realtime)
}

// ConfigLoader keeps its lookup, and the caller keeps the environment table
struct Borrowed<'environment>(&'environment dyn EnvironmentLookup);

impl EnvironmentLookup for Borrowed<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.lookup(name)
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.0.system_home()
    }
}
