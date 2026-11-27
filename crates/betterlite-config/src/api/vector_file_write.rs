use std::path::PathBuf;

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};

mod tests;

// REQUEST_TIMEOUT, which LiteLLM's constants module reads once at import, 6000 seconds when the environment leaves it unset
pub fn loads_write_timeout(environment: &dyn EnvironmentLookup) -> Result<f64, ConfigError> {
    let config = ConfigLoader {
        lookup: Borrowed(environment),
    }
    .load("litellm_settings: {}")?;
    Ok(config.sections.routing_timeouts.global.seconds)
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
