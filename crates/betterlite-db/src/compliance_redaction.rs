use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

mod tests;

const REDACTED_VALUE: &str = "[REDACTED]";

// Mirrors LiteLLM's fn redact_nested_match_and_regex_keys: payload is None or text and no Exception escapes, so a value that is not an object or list comes back as it was
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

#[must_use]
pub fn redact_match_and_regex_keys(payload: &Json, keys: &RedactionKeys) -> Json {
    let mut redacted = payload.clone();
    {
        let mut stack = vec![&mut redacted];
        // an owned Json tree cannot reference itself, so no node is reachable twice and no seen set is needed
        while let Some(node) = stack.pop() {
            match node {
                Json::Object(fields) => {
                    for (name, value) in fields.iter_mut() {
                        if keys.names.iter().any(|key| key == name) {
                            *value = Json::Text(REDACTED_VALUE.to_owned());
                        } else {
                            stack.push(value);
                        }
                    }
                }
                Json::List(items) => stack.extend(items.iter_mut()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
            }
        }
    }
    redacted
}
