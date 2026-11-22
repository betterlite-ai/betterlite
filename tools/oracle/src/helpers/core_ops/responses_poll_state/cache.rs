use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct ScriptedCache {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    pub(super) effects: Mutex<Vec<Value>>,
}

impl ScriptedCache {
    pub(super) fn seed_bytes(&self, key: &str, value: &[u8]) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_vec());
    }

    pub(super) fn effect_count(&self) -> usize {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn record(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }
}

#[async_trait]
impl PollingCache for ScriptedCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        self.record(json!({"op": "read", "key": key, "ttl": null, "value": null}));
        let value = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        Ok(value)
    }

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError> {
        let parsed = serde_json::from_str::<Value>(value)
            .unwrap_or_else(|_source| Value::String(value.to_owned()));
        self.record(
            json!({"op": "write", "key": key, "ttl": ttl, "value": public_projection(parsed)}),
        );
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.as_bytes().to_vec());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        self.record(json!({"op": "delete", "key": key, "ttl": null, "value": null}));
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(i64::from(entries.shift_remove(key).is_some()))
    }
}

// the owner rides in betterlite's private envelope; the helper compares only the response
fn public_projection(value: Value) -> Value {
    let Value::Object(mut fields) = value else {
        return value;
    };
    let envelope =
        fields.len() == 2 && fields.contains_key("owner") && fields.contains_key("response");
    if envelope {
        return fields.remove("response").unwrap_or(Value::Null);
    }
    Value::Object(fields)
}
