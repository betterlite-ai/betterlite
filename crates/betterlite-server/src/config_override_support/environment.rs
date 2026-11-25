use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::models::OverrideKind;

// A typed set/remove batch: Some(value) sets a mapped env var, None removes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentBatch {
    pub changes: BTreeMap<String, Option<String>>,
}

// The effective values at one moment; later runtime batches never change it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentSnapshot {
    pub values: BTreeMap<String, String>,
}

// The deployment's boot values under the overrides applied since, never the process environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeEnvironment {
    pub boot: BTreeMap<String, String>,
    pub overlay: BTreeMap<String, Option<String>>,
}

impl RuntimeEnvironment {
    pub fn apply(&mut self, batch: &EnvironmentBatch) {
        for (env_var, value) in &batch.changes {
            self.overlay.insert(env_var.clone(), value.clone());
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> EnvironmentSnapshot {
        let mut values = self.boot.clone();
        for (env_var, value) in &self.overlay {
            match value {
                Some(text) => {
                    values.insert(env_var.clone(), text.clone());
                }
                None => {
                    values.remove(env_var);
                }
            }
        }
        EnvironmentSnapshot { values }
    }
}

impl EnvironmentLookup for RuntimeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.overlay
            .get(name)
            .map_or_else(|| self.boot.get(name).cloned(), Clone::clone)
    }
}

impl EnvironmentLookup for EnvironmentSnapshot {
    fn lookup(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}

// LiteLLM's _set_env_vars: a missing, null or empty field clears its env var, any other value is stringified.
#[must_use]
pub fn project_environment(kind: OverrideKind, config: &JsonMap) -> EnvironmentBatch {
    let changes = kind
        .fields()
        .iter()
        .map(|field| {
            let value = match config.get(field.name) {
                Some(JsonValue::String(text)) if text.is_empty() => None,
                Some(JsonValue::String(text)) => Some(text.clone()),
                Some(
                    other @ (JsonValue::Bool(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_)),
                ) => Some(other.to_string()),
                Some(JsonValue::Null) | None => None,
            };
            (field.env_var.to_owned(), value)
        })
        .collect();
    EnvironmentBatch { changes }
}
