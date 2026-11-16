use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

mod tests;

const REDACTED_VALUE: &str = "[REDACTED]";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RedactionKeys {
    pub names: Vec<String>,
}

impl Default for RedactionKeys {
    fn default() -> Self {
        Self {
            names: vec![String::from("match"), String::from("regex")],
        }
    }
}

// an owned Json clone can neither fail nor be None, so LiteLLM's early return for payload is None and its copy Exception catch have no counterpart here
#[must_use]
// ast-grep-ignore: no-long-names
pub fn redact_nested_match_and_regex_keys(payload: &Json, keys: &RedactionKeys) -> Json {
    let mut redacted = payload.clone();
    let mut stack = vec![&mut redacted];
    // an owned Json tree cannot reference itself, so no node is reachable twice and no seen set is needed
    while let Some(node) = stack.pop() {
        let object = match node {
            Json::Object(fields) => fields,
            Json::List(items) => {
                stack.extend(items.iter_mut());
                continue;
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => continue,
        };
        for (name, value) in object.iter_mut() {
            if keys.names.iter().any(|key| key == name) {
                *value = Json::Text(REDACTED_VALUE.to_owned());
            } else {
                stack.push(value);
            }
        }
    }
    redacted
}
