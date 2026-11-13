use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::loader::EnvironmentLookup;

pub struct HomeEnvironment {
    pub variables: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
}

impl EnvironmentLookup for HomeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}
