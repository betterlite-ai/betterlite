use std::path::Path;

use crate::sections::includes;

pub type ConfigMap = includes::ConfigMap;
pub type IncludeError = includes::IncludeError;

pub fn resolve_includes(config: ConfigMap, path: &Path) -> Result<ConfigMap, Box<IncludeError>> {
    match includes::resolve_includes(config, path) {
        Ok(resolved) => Ok(resolved),
        Err(error) => Err(Box::new(error)),
    }
}
