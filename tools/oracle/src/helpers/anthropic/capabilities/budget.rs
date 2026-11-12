use betterlite_anthropic::capabilities::budget::cap_thinking_to_max_tokens;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct CapStep {
    thinking: IndexMap<String, Json>,
    max_tokens: Option<i64>,
}

impl CapStep {
    pub(super) fn answer(&self) -> Result<Value, Box<HelperError>> {
        cap_thinking_to_max_tokens(&self.thinking, self.max_tokens)
            .map_or(Ok(Value::Null), |thinking| {
                serde_json::to_value(thinking).or_value_error()
            })
    }
}
