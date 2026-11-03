pub mod files;
pub mod loading;
pub mod model;
pub mod paths;
pub mod pattern;

#[path = "content_category/tests.rs"]
mod tests;

#[path = "content_category/category_config/tests.rs"]
mod category_config;
#[path = "content_category/conditional/tests.rs"]
mod conditional;
#[path = "content_category/file/tests.rs"]
mod file;
#[path = "content_category/load_categories/tests.rs"]
mod load_categories;
#[path = "content_category/partial_updates/tests.rs"]
mod partial_updates;
#[path = "content_category/path_assertion/tests.rs"]
mod path_assertion;
#[path = "content_category/path_resolution/tests.rs"]
mod path_resolution;
#[path = "content_category/severity/tests.rs"]
mod severity;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::{
    Section,
    settings::{JsonValue, coerce_bool::coerce_bool},
};

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub allow_external_paths: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let value = lookup
            .lookup("LITELLM_CONTENT_FILTER_ALLOW_EXTERNAL_PATHS")
            .unwrap_or_default();
        Ok(Settings {
            allow_external_paths: matches!(
                coerce_bool(JsonValue::String(value)),
                JsonValue::Bool(true)
            ),
        })
    }
}
