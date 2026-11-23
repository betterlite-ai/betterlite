use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_provider::azure_dispatch::call::{Composition, Globals};

// the one Azure composition a server builds; its section resolved every value at load, so no request reads the environment
#[must_use]
pub fn build(config: &Config) -> Composition {
    let section = &config.sections.azure_provider_globals;
    let mut composition = Composition::from_environment(section.environment.clone());
    composition.globals = Globals {
        api_base: section.globals.api_base.clone(),
        api_version: section.globals.api_version.clone(),
        api_key: section.globals.api_key.clone(),
        azure_key: section.globals.azure_key.clone(),
        headers: section.globals.headers.clone(),
        // the endpoints section already resolves litellm_settings.AZURE_DEFAULT_API_VERSION and its default
        azure_default_api_version: Some(
            config
                .sections
                .azure_endpoints
                .azure_default_api_version
                .clone(),
        ),
    };
    composition
}

pub struct Slot {
    pub composition: Arc<Composition>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self {
            composition: Arc::new(build(config)),
        }
    }
}

mod tests;
