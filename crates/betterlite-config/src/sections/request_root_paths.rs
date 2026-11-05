use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const ROOT_PATH_NAME: &str = "SERVER_ROOT_PATH";
const ROOT_PATHS_NAME: &str = "SERVER_ROOT_PATHS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "SERVER_ROOT_PATH")]
    server_root_path: Option<String>,
    #[serde(default, rename = "SERVER_ROOT_PATHS")]
    server_root_paths: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub server_root_path: String,
    // Raw comma-separated entries; the server normalizes them once when it installs the middleware.
    pub server_root_paths: Vec<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = |value: Option<String>, name: &str| match value {
            Some(text) => resolve_string(text, &format!("general_settings.{name}"), lookup),
            None => Ok(lookup.lookup(name).unwrap_or_default()),
        };
        let root_paths = configured(self.general_settings.server_root_paths, ROOT_PATHS_NAME)?;
        Ok(Settings {
            server_root_path: configured(self.general_settings.server_root_path, ROOT_PATH_NAME)?,
            server_root_paths: if root_paths.trim().is_empty() {
                Vec::new()
            } else {
                root_paths.split(',').map(str::to_owned).collect()
            },
        })
    }
}
