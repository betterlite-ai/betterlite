use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use serde_json::Value;

#[derive(Default)]
pub(super) struct ScriptedCache {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    effects: Mutex<Vec<Effect>>,
}

#[derive(Clone)]
pub(super) struct Effect {
    pub(super) operation: &'static str,
    pub(super) key: String,
    pub(super) ttl: Option<i64>,
    pub(super) value: Option<Value>,
}

impl ScriptedCache {
    pub(super) fn seed(&self, key: &str, value: &str) {
        self.seed_bytes(key, value.as_bytes());
    }

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

    pub(super) fn effects_since(&self, start: usize) -> Vec<Effect> {
        let effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        effects.iter().skip(start).cloned().collect()
    }

    fn record(&self, effect: Effect) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }
}

#[async_trait]
impl PollingCache for ScriptedCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let value = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        self.record(Effect {
            operation: "read",
            key: key.to_owned(),
            ttl: None,
            value: None,
        });
        Ok(value)
    }

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError> {
        let parsed = match serde_json::from_str::<Value>(value) {
            Ok(parsed) => parsed,
            Err(_source) => Value::String(value.to_owned()),
        };
        self.record(Effect {
            operation: "write",
            key: key.to_owned(),
            ttl: Some(ttl),
            value: Some(public_projection(parsed)),
        });
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.as_bytes().to_vec());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        let removed = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        self.record(Effect {
            operation: "delete",
            key: key.to_owned(),
            ttl: None,
            value: None,
        });
        Ok(if removed.is_some() { 1 } else { 0 })
    }
}

// the owner rides in betterlite's private envelope; the helper compares only the response
fn public_projection(value: Value) -> Value {
    match value {
        Value::Object(mut fields) => {
            let envelope = fields.len() == 2
                && fields.contains_key("owner")
                && fields.contains_key("response");
            if envelope {
                fields.remove("response").unwrap_or(Value::Null)
            } else {
                Value::Object(fields)
            }
        }
        other => other,
    }
}
