use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) struct Op;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub values: Vec<Value>,
    #[serde(default)]
    pub callbacks: Vec<Value>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut counts = std::collections::BTreeMap::new();
        for value in input.values.iter().chain(input.callbacks.iter()) {
            let class = if value
                .as_str()
                .is_some_and(|text| text.starts_with("v2:gcm:"))
            {
                "migrated"
            } else if value.is_string() {
                "plaintext"
            } else {
                "not-a-string"
            };
            *counts.entry(class).or_insert(0_u64) += 1;
        }
        Ok(serde_json::json!({"counts": counts}))
    }
}
