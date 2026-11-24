use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

// LiteLLM's _ConfigWithBaseline: the config a save writes and the baseline it diffs against, absent when the caller has only the values
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigWithBaseline {
    pub values: BTreeMap<String, JsonValue>,
    pub(crate) baseline: Option<BTreeMap<String, JsonValue>>,
}

impl ConfigWithBaseline {
    #[must_use]
    pub fn from_values(values: BTreeMap<String, JsonValue>) -> Self {
        Self {
            values,
            baseline: None,
        }
    }

    #[must_use]
    pub fn with_baseline(
        values: BTreeMap<String, JsonValue>,
        baseline: BTreeMap<String, JsonValue>,
    ) -> Self {
        Self {
            values,
            baseline: Some(baseline),
        }
    }
}
