use serde_json::{Map, Value};

use super::error::RouterSettingsError;
use crate::retry_policy::policy::RetryPolicy;

// None keeps the stored policy: LiteLLM drops a value that is neither an object nor null
pub fn retry_policy_value(value: &Value) -> Result<Option<Value>, RouterSettingsError> {
    let fields = match value {
        Value::Null => return Ok(Some(Value::Null)),
        Value::Object(fields) => fields,
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => return Ok(None),
    };
    let unset = serde_json::to_value(RetryPolicy::default()).map_err(|source| {
        RouterSettingsError::RetryPolicy {
            field: "retry_policy".to_owned(),
            source,
        }
    })?;
    let mut policy: Map<String, Value> = unset.as_object().cloned().unwrap_or_default();
    for (field, retries) in fields {
        if !policy.contains_key(field) {
            continue;
        }
        serde_json::from_value::<Option<i64>>(retries.clone()).map_err(|source| {
            RouterSettingsError::RetryPolicy {
                field: field.clone(),
                source,
            }
        })?;
        policy.insert(field.clone(), retries.clone());
    }
    Ok(Some(Value::Object(policy)))
}
