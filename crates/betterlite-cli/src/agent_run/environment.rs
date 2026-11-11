use crate::agent_launch::EnvVars;
use betterlite_config::loader::EnvironmentLookup;
use std::path::{Path, PathBuf};
pub struct BaseEnvironment<'environment> {
    pub variables: &'environment EnvVars,
    pub home: Option<&'environment Path>,
}
impl EnvironmentLookup for BaseEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }
    fn system_home(&self) -> Option<PathBuf> {
        self.home.map(Path::to_path_buf)
    }
}
