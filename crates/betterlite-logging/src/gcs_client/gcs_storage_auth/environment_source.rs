use std::collections::BTreeMap;

// LiteLLM reads GOOGLE_SECRET_MANAGER_PROJECT_ID at each call, so the base asks this source instead of caching a value
pub trait EnvironmentSource: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;

    fn snapshot(&self) -> BTreeMap<String, String>;
}

impl EnvironmentSource for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        self.clone()
    }
}

mod tests;
