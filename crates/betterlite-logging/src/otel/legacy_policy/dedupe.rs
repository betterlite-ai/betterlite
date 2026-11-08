use std::collections::{BTreeMap, BTreeSet};
use std::num::FpCategory;

use crate::otel::model::utils::coercion::OtelModelValue;

mod tests;

pub const MAX_FREEZE_DEPTH: u32 = 16;

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum ScopeValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Set(Vec<Self>),
    Map(BTreeMap<String, Self>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
pub struct FloatKey(u64);

impl From<f64> for FloatKey {
    fn from(value: f64) -> Self {
        if value.classify() == FpCategory::Zero {
            return Self(0.0_f64.to_bits());
        }
        Self(value.to_bits())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FrozenScope {
    Null,
    Bool(bool),
    Int(i64),
    Float(FloatKey),
    Text(String),
    List(Vec<Self>),
    Set(BTreeSet<Self>),
    Map(BTreeMap<String, Self>),
    Json(String),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct HandlerIdentity {
    pub class_name: String,
    pub instance: u64,
}

type EmitKey = (String, u64, Vec<FrozenScope>);

#[derive(Debug, Default)]
pub struct EmittedSpans {
    seen: BTreeSet<EmitKey>,
}

impl EmittedSpans {
    pub fn emit_once(&mut self, handler: &HandlerIdentity, scope: &[ScopeValue]) -> bool {
        let frozen = scope.iter().map(|value| freeze_scope(value, 0)).collect();
        self.seen
            .insert((handler.class_name.clone(), handler.instance, frozen))
    }
}

#[must_use]
pub fn freeze_scope(value: &ScopeValue, depth: u32) -> FrozenScope {
    if depth >= MAX_FREEZE_DEPTH {
        // past the depth limit the remaining value keeps its compact JSON shape
        return FrozenScope::Json(serde_json::to_string(&scope_json(value)).unwrap_or_default());
    }
    let next = depth.saturating_add(1);
    match value {
        ScopeValue::Null => FrozenScope::Null,
        ScopeValue::Bool(flag) => FrozenScope::Bool(*flag),
        ScopeValue::Int(number) => FrozenScope::Int(*number),
        ScopeValue::Float(number) => FrozenScope::Float(FloatKey::from(*number)),
        ScopeValue::Text(text) => FrozenScope::Text(text.clone()),
        ScopeValue::List(items) => {
            FrozenScope::List(items.iter().map(|item| freeze_scope(item, next)).collect())
        }
        ScopeValue::Set(items) => {
            FrozenScope::Set(items.iter().map(|item| freeze_scope(item, next)).collect())
        }
        ScopeValue::Map(fields) => FrozenScope::Map(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), freeze_scope(item, next)))
                .collect(),
        ),
    }
}

fn scope_json(value: &ScopeValue) -> OtelModelValue {
    match value {
        ScopeValue::Null => OtelModelValue::Null(()),
        ScopeValue::Bool(flag) => OtelModelValue::Bool(*flag),
        ScopeValue::Int(number) => OtelModelValue::Number(serde_json::Number::from(*number)),
        ScopeValue::Float(number) => serde_json::Number::from_f64(*number)
            .map_or(OtelModelValue::Null(()), OtelModelValue::Number),
        ScopeValue::Text(text) => OtelModelValue::String(text.clone()),
        ScopeValue::List(items) | ScopeValue::Set(items) => {
            OtelModelValue::Array(items.iter().map(scope_json).collect())
        }
        ScopeValue::Map(fields) => OtelModelValue::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), scope_json(item)))
                .collect(),
        ),
    }
}
