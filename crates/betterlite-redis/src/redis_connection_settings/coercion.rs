mod tests;

use std::collections::BTreeMap;

use super::model::scalars::{
    BoolOrText, CoercionKind, FloatOrText, IntOrText, PassthroughValue, str_to_bool,
};

// litellm._redis._coerce_redis_kwargs_types's explicit_param_types: picks the target type directly, instead of inferring it from a signature default.
const PASSTHROUGH_COERCION: &[(&str, CoercionKind)] = &[
    ("cluster_error_retry_attempts", CoercionKind::Int),
    ("reinitialize_steps", CoercionKind::Int),
    ("connection_error_retry_attempts", CoercionKind::Int),
    ("require_full_coverage", CoercionKind::Bool),
    ("read_from_replicas", CoercionKind::Bool),
    ("dynamic_startup_nodes", CoercionKind::Bool),
    ("ssl_check_hostname", CoercionKind::Bool),
    ("decode_responses", CoercionKind::Bool),
];

#[must_use]
pub fn coerce_int(value: Option<IntOrText>) -> Option<i64> {
    match value? {
        IntOrText::Int(parsed) => Some(parsed),
        IntOrText::Text(text) => text.parse().ok(),
    }
}

#[must_use]
pub fn coerce_float(value: Option<FloatOrText>) -> Option<f64> {
    match value? {
        FloatOrText::Float(parsed) => Some(parsed),
        FloatOrText::Text(text) => text.parse().ok(),
    }
}

// _str_to_bool never fails: any string that is not "true"/"1"/"yes" coerces to false.
#[must_use]
pub fn coerce_bool(value: Option<BoolOrText>) -> Option<bool> {
    match value? {
        BoolOrText::Bool(parsed) => Some(parsed),
        BoolOrText::Text(text) => Some(str_to_bool(&text)),
    }
}

// A failed int()/float() conversion deletes the key outright in litellm; bool never fails.
pub fn coerce_passthrough(passthrough: &mut BTreeMap<String, PassthroughValue>) {
    for (name, kind) in PASSTHROUGH_COERCION {
        let Some(PassthroughValue::Text(text)) = passthrough.get(*name) else {
            continue;
        };
        match coerce_passthrough_value(*kind, text) {
            Some(coerced) => {
                passthrough.insert((*name).to_owned(), coerced);
            }
            None => {
                passthrough.remove(*name);
            }
        }
    }
}

fn coerce_passthrough_value(kind: CoercionKind, text: &str) -> Option<PassthroughValue> {
    match kind {
        CoercionKind::Int => text
            .parse::<i64>()
            .ok()
            .map(|parsed| PassthroughValue::Number(serde_json::Number::from(parsed))),
        CoercionKind::Float => {
            serde_json::Number::from_f64(text.parse::<f64>().ok()?).map(PassthroughValue::Number)
        }
        CoercionKind::Bool => Some(PassthroughValue::Bool(str_to_bool(text))),
    }
}
