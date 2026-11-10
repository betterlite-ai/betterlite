use std::collections::BTreeMap;

pub trait EnvironmentSource {
    fn lookup(&self, name: &str) -> Option<String>;
}

impl EnvironmentSource for BTreeMap<String, String> {
    fn lookup(&self, name: &str) -> Option<String> {
        // LiteLLM's settings model folds names to one case, so every spelling of an alias names the same variable
        self.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }
}
