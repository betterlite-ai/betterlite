pub mod aiml;
pub mod aliases;
pub mod baseten;
pub mod cerebras;
pub mod error;
pub mod featherless;

mod tests;

use indexmap::IndexMap;
use serde_json::Value;

// Baseten and Cerebras walk the caller's order, rename the token limit even when null, and copy supported values as given
pub fn map_renaming_token_limit(
    non_default: &IndexMap<String, Value>,
    existing: &mut IndexMap<String, Value>,
    supported: &[&str],
) {
    for (name, value) in non_default {
        if name == "max_completion_tokens" {
            existing.insert(String::from("max_tokens"), value.clone());
        } else if supported.contains(&name.as_str()) {
            existing.insert(name.clone(), value.clone());
        }
    }
}
