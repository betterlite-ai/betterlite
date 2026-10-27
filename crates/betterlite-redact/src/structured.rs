#![expect(
    clippy::disallowed_types,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "B244 owns the existing opaque JSON payload field and its typed traversal boundary"
)]

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

const DEFAULT_PATTERNS: &str = "password secret key token auth authorization cookie credential credentials access private certificate fingerprint";
const DEFAULT_MASK_DEPTH: usize = 10;
const DEFAULT_REDACT_DEPTH: usize = 100;
const MAX_WORK: usize = 1_000_000;
pub const REDACTED: &str = "REDACTED";
pub type Payload = Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensitiveDataMasker {
    sensitive_patterns: BTreeSet<String>,
    non_sensitive_overrides: BTreeSet<String>,
    visible_prefix: usize,
    visible_suffix: usize,
    mask_char: char,
    mask_short_values: bool,
}

impl SensitiveDataMasker {
    #[must_use]
    pub fn new() -> Self {
        let extra_patterns = Vec::<String>::new();
        let overrides = ["cost"];
        Self::with_patterns(extra_patterns, overrides)
    }

    pub fn with_patterns<I, J>(extra_patterns: I, overrides: J) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
        J: IntoIterator,
        J::Item: AsRef<str>,
    {
        let mut sensitive_patterns = DEFAULT_PATTERNS
            .split_whitespace()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        sensitive_patterns.extend(
            extra_patterns
                .into_iter()
                .map(|item| item.as_ref().to_owned()),
        );
        let non_sensitive_overrides = overrides
            .into_iter()
            .map(|item| item.as_ref().to_owned())
            .collect();
        Self {
            sensitive_patterns,
            non_sensitive_overrides,
            visible_prefix: 4,
            visible_suffix: 4,
            mask_char: '*',
            mask_short_values: true,
        }
    }

    fn error_masker() -> Self {
        let mut masker = Self::new();
        masker.visible_suffix = 0;
        masker
    }

    #[must_use]
    pub fn mask_value(&self, value: &str) -> String {
        let characters = value.chars().collect::<Vec<_>>();
        if characters.is_empty() {
            return String::new();
        }
        let visible = self.visible_prefix.saturating_add(self.visible_suffix);
        if characters.len() <= visible {
            return if self.mask_short_values {
                self.mask_char.to_string().repeat(characters.len())
            } else {
                value.to_owned()
            };
        }
        let masked_length = characters.len().saturating_sub(visible);
        let prefix = characters
            .iter()
            .take(self.visible_prefix)
            .collect::<String>();
        let suffix = characters
            .iter()
            .rev()
            .take(self.visible_suffix)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!(
            "{prefix}{}{suffix}",
            self.mask_char.to_string().repeat(masked_length)
        )
    }

    #[must_use]
    pub fn is_sensitive_key(&self, key: &str, excluded_keys: &[String]) -> bool {
        if excluded_keys.iter().any(|excluded| excluded == key) {
            return false;
        }
        let normalized_key = key.to_ascii_lowercase().replace('-', "_");
        let segments = normalized_key.split('_').collect::<BTreeSet<_>>();
        if self
            .non_sensitive_overrides
            .iter()
            .any(|item| segments.contains(item.as_str()))
        {
            return false;
        }
        self.sensitive_patterns
            .iter()
            .any(|item| segments.contains(item.as_str()))
    }

    #[must_use]
    pub fn mask_dict(
        &self,
        data: &Map<String, Value>,
        depth: usize,
        max_depth: usize,
        excluded_keys: &[String],
    ) -> Value {
        if depth >= max_depth {
            return Value::String(REDACTED.to_owned());
        }
        let mut masked = Map::new();
        for (key, value) in data {
            let sensitive = self.is_sensitive_key(key, excluded_keys);
            masked.insert(
                key.clone(),
                self.mask_nested(
                    value,
                    depth.saturating_add(1),
                    max_depth,
                    excluded_keys,
                    sensitive,
                ),
            );
        }
        Value::Object(masked)
    }

    fn mask_nested(
        &self,
        value: &Value,
        depth: usize,
        max_depth: usize,
        excluded_keys: &[String],
        sensitive: bool,
    ) -> Value {
        match value {
            Value::Object(object) => self.mask_dict(object, depth, max_depth, excluded_keys),
            Value::Array(_) if depth >= max_depth => Value::String(REDACTED.to_owned()),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|item| {
                        self.mask_nested(
                            item,
                            depth.saturating_add(1),
                            max_depth,
                            excluded_keys,
                            sensitive,
                        )
                    })
                    .collect(),
            ),
            _ if sensitive => Value::String(self.mask_value(&scalar_text(value))),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
        }
    }

    #[must_use]
    pub fn mask(&self, data: &Value) -> Value {
        match data {
            Value::Object(object) => self.mask_dict(object, 0, DEFAULT_MASK_DEPTH, &[]),
            Value::Array(_)
            | Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_) => self.mask_nested(data, 0, DEFAULT_MASK_DEPTH, &[], false),
        }
    }
}

impl Default for SensitiveDataMasker {
    fn default() -> Self {
        Self::new()
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(string) => string.clone(),
    }
}

#[must_use]
pub fn mask_sensitive_structure(data: &Value) -> Value {
    let masker = SensitiveDataMasker::error_masker();
    if data.is_null() {
        Value::Null
    } else {
        masker.mask(data)
    }
}

#[must_use]
pub fn mask_credentials_in_payload(data: &Value) -> Value {
    let mut walker = PayloadWalker {
        memo: HashMap::new(),
        work: 0,
    };
    walker.walk(data, false, 0)
}

struct PayloadWalker {
    memo: HashMap<(String, bool), Value>,
    work: usize,
}

impl PayloadWalker {
    fn walk(&mut self, node: &Value, sensitive: bool, depth: usize) -> Value {
        self.work = self.work.saturating_add(1);
        if self.work > MAX_WORK {
            return Value::String(REDACTED.to_owned());
        }
        if let Value::String(value) = node {
            return if sensitive {
                Value::String(SensitiveDataMasker::new().mask_value(value))
            } else {
                node.clone()
            };
        }
        if !node.is_object() && !node.is_array() {
            return node.clone();
        }
        if depth >= DEFAULT_MASK_DEPTH {
            return Value::String(REDACTED.to_owned());
        }
        let cache_key = (node.to_string(), sensitive);
        if let Some(value) = self.memo.get(&cache_key) {
            return value.clone();
        }
        self.memo
            .insert(cache_key.clone(), Value::String(REDACTED.to_owned()));
        let value = match node {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            self.walk(
                                value,
                                SensitiveDataMasker::new().is_sensitive_key(key, &[]),
                                depth.saturating_add(1),
                            ),
                        )
                    })
                    .collect(),
            ),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| self.walk(value, sensitive, depth.saturating_add(1)))
                    .collect(),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => node.clone(),
        };
        self.memo.insert(cache_key, value.clone());
        value
    }
}

pub mod redact;
mod tests;
