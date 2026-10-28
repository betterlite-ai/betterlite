#![expect(
    clippy::disallowed_types,
    reason = "B244 owns the existing opaque JSON payload field and its typed traversal boundary"
)]

use std::collections::BTreeSet;

use serde_json::{Map, Value};

const DEFAULT_PATTERNS: &str = "password secret key token auth authorization cookie credential credentials access private certificate fingerprint tenancy";
const DEFAULT_MASK_DEPTH: usize = 10;
const DEFAULT_REDACT_DEPTH: usize = 100;
const MAX_WORK: usize = 1_000_000;
pub const REDACTED: &str = "REDACTED";
pub type Payload = Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensitiveDataMasker {
    pub sensitive_patterns: BTreeSet<String>,
    pub non_sensitive_overrides: BTreeSet<String>,
    pub visible_prefix: usize,
    pub visible_suffix: usize,
    pub mask_char: String,
    pub mask_short_values: bool,
    pub max_work: usize,
}

impl SensitiveDataMasker {
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
        let mut non_sensitive_overrides = overrides
            .into_iter()
            .map(|item| item.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        if non_sensitive_overrides.is_empty() {
            non_sensitive_overrides.insert("cost".to_owned());
        }
        Self {
            sensitive_patterns,
            non_sensitive_overrides,
            visible_prefix: 4,
            visible_suffix: 4,
            mask_char: "*".to_owned(),
            mask_short_values: true,
            max_work: MAX_WORK,
        }
    }

    #[must_use]
    pub fn mask_value(&self, text: &str) -> String {
        if text.len() > self.max_work {
            REDACTED.to_owned()
        } else {
            value::mask(self, text)
        }
    }

    #[must_use]
    pub fn is_sensitive_key(&self, key: &str, excluded_keys: &[String]) -> bool {
        if excluded_keys.iter().any(|excluded| excluded == key) {
            return false;
        }
        keys::sensitive(self, key)
    }

    #[must_use]
    pub fn mask_dict(
        &self,
        data: &Map<String, Value>,
        depth: usize,
        max_depth: usize,
        excluded_keys: &[String],
    ) -> Value {
        let mut walker = walk::Walker {
            masker: self,
            kind: walk::Kind::Config,
            excluded_keys,
            max_depth: max_depth.min(DEFAULT_REDACT_DEPTH),
            remaining: self.max_work,
        };
        walker
            .mapping(data, depth)
            .unwrap_or_else(|| Value::String(REDACTED.to_owned()))
    }

    #[must_use]
    pub fn mask(&self, data: &Value) -> Value {
        if let Value::Object(mapping) = data {
            return self.mask_dict(mapping, 0, DEFAULT_MASK_DEPTH, &[]);
        }
        let mut walker = walk::Walker {
            masker: self,
            kind: walk::Kind::Config,
            excluded_keys: &[],
            max_depth: DEFAULT_MASK_DEPTH,
            remaining: self.max_work,
        };
        walker
            .walk(data, 0, walk::Context::Root)
            .unwrap_or_else(|| Value::String(REDACTED.to_owned()))
    }
}

impl Default for SensitiveDataMasker {
    fn default() -> Self {
        Self::with_patterns(Vec::<String>::new(), ["cost"])
    }
}

#[must_use]
pub fn mask_sensitive_structure(data: &Value) -> Value {
    let defaults = SensitiveDataMasker::default();
    SensitiveDataMasker {
        sensitive_patterns: defaults.sensitive_patterns,
        non_sensitive_overrides: defaults.non_sensitive_overrides,
        visible_prefix: defaults.visible_prefix,
        visible_suffix: 0,
        mask_char: defaults.mask_char,
        mask_short_values: defaults.mask_short_values,
        max_work: defaults.max_work,
    }
    .mask(data)
}

#[must_use]
pub fn mask_credentials_in_payload(data: &Value) -> Value {
    let masker = SensitiveDataMasker::default();
    let mut walker = walk::Walker {
        masker: &masker,
        kind: walk::Kind::Payload,
        excluded_keys: &[],
        max_depth: DEFAULT_MASK_DEPTH,
        remaining: MAX_WORK,
    };
    walker
        .walk(data, 0, walk::Context::Root)
        .unwrap_or_else(|| Value::String(REDACTED.to_owned()))
}

mod keys;
mod number;
pub mod parse;
pub mod redact;
mod tests;
mod value;
mod walk;
