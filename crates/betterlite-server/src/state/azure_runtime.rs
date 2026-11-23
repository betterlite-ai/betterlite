use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::azure_provider_globals::{
    AD_TOKEN_VARIABLE, API_BASE_VARIABLE, API_KEY_VARIABLE, API_TYPE_VARIABLE,
    API_VERSION_VARIABLE, Environment, OPENAI_API_KEY_VARIABLE,
};
use betterlite_config::types::Config;
use betterlite_provider::azure_dispatch::call::{Composition, Globals};
use secrecy::ExposeSecret as _;

// the one Azure composition a server builds; its section resolved every value at load, so no request reads the environment
#[must_use]
pub fn build(config: &Config) -> Composition {
    let section = &config.sections.azure_provider_globals;
    let mut composition = Composition::from_environment(plain_environment(&section.environment));
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

// the composition looks variables up in a plain map, which this copies out of the typed section once at startup
fn plain_environment(environment: &Environment) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    if let Some(value) = &environment.api_base {
        entries.insert(API_BASE_VARIABLE.to_owned(), value.clone());
    }
    if let Some(value) = &environment.api_version {
        entries.insert(API_VERSION_VARIABLE.to_owned(), value.clone());
    }
    if let Some(value) = &environment.api_key {
        entries.insert(
            API_KEY_VARIABLE.to_owned(),
            value.expose_secret().to_owned(),
        );
    }
    if let Some(value) = &environment.openai_api_key {
        entries.insert(
            OPENAI_API_KEY_VARIABLE.to_owned(),
            value.expose_secret().to_owned(),
        );
    }
    if let Some(value) = &environment.ad_token {
        entries.insert(
            AD_TOKEN_VARIABLE.to_owned(),
            value.expose_secret().to_owned(),
        );
    }
    if let Some(value) = &environment.api_type {
        entries.insert(API_TYPE_VARIABLE.to_owned(), value.clone());
    }
    entries
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
