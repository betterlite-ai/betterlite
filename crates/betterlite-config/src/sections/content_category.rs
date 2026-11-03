pub mod files;
pub mod loading;
pub mod model;
pub mod paths;

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
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw;

#[derive(Debug, Clone, Default)]
pub struct Settings;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings)
    }
}
