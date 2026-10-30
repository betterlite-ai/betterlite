mod tests;

use std::collections::BTreeMap;

use super::model::scalars::{BoolOrText, CoercionKind, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::{ClientKind, Target};

#[must_use]
pub fn coerce_int(value: Option<IntOrText>) -> Option<i64> {
    match value? {
        IntOrText::Int(parsed) => Some(parsed),
        IntOrText::Text(text) => text.trim().replace('_', "").parse().ok(),
    }
}

#[must_use]
pub fn coerce_float(value: Option<FloatOrText>) -> Option<f64> {
    match value? {
        FloatOrText::Float(parsed) => Some(parsed),
        FloatOrText::Text(text) => text.trim().replace('_', "").parse().ok(),
    }
}

// _str_to_bool never fails: any string that is not "true"/"1"/"yes" coerces to false.
#[must_use]
pub fn coerce_bool(value: Option<BoolOrText>) -> Option<bool> {
    match value? {
        BoolOrText::Bool(parsed) => Some(parsed),
        BoolOrText::Text(text) => {
            Some(matches!(text.to_lowercase().as_str(), "true" | "1" | "yes"))
        }
    }
}

// A failed int()/float() conversion deletes the key outright in litellm; bool never fails.
pub fn coerce_passthrough(
    target: Target,
    _client: ClientKind,
    passthrough: &mut BTreeMap<String, PassthroughValue>,
) {
    for (name, kind) in coercion_table(target) {
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
            .trim()
            .replace('_', "")
            .parse::<i64>()
            .ok()
            .map(|parsed| PassthroughValue::Number(serde_json::Number::from(parsed))),
        CoercionKind::Float => {
            serde_json::Number::from_f64(text.trim().replace('_', "").parse::<f64>().ok()?)
                .map(PassthroughValue::Number)
        }
        CoercionKind::Bool => Some(PassthroughValue::Bool(matches!(
            text.to_lowercase().as_str(),
            "true" | "1" | "yes"
        ))),
    }
}

// This table mirrors the bool/int/float defaults in the pinned redis.Redis, Connection and RedisCluster signatures.
const fn coercion_table(target: Target) -> &'static [(&'static str, CoercionKind)] {
    const CLIENT: &[(&str, CoercionKind)] = &[
        ("retry_on_timeout", CoercionKind::Bool),
        ("single_connection_client", CoercionKind::Bool),
        ("decode_responses", CoercionKind::Bool),
        ("health_check_interval", CoercionKind::Int),
        ("protocol", CoercionKind::Int),
        ("socket_read_size", CoercionKind::Int),
        ("socket_type", CoercionKind::Int),
        ("auto_close_connection_pool", CoercionKind::Bool),
        ("legacy_responses", CoercionKind::Bool),
        ("ssl_check_hostname", CoercionKind::Bool),
        ("ssl_validate_ocsp", CoercionKind::Bool),
        ("ssl_validate_ocsp_stapled", CoercionKind::Bool),
    ];
    const CLUSTER: &[(&str, CoercionKind)] = &[
        ("retry_on_timeout", CoercionKind::Bool),
        ("single_connection_client", CoercionKind::Bool),
        ("decode_responses", CoercionKind::Bool),
        ("health_check_interval", CoercionKind::Int),
        ("protocol", CoercionKind::Int),
        ("ssl_check_hostname", CoercionKind::Bool),
        ("ssl_validate_ocsp", CoercionKind::Bool),
        ("ssl_validate_ocsp_stapled", CoercionKind::Bool),
        ("cluster_error_retry_attempts", CoercionKind::Int),
        ("reinitialize_steps", CoercionKind::Int),
        ("connection_error_retry_attempts", CoercionKind::Int),
        ("require_full_coverage", CoercionKind::Bool),
        ("read_from_replicas", CoercionKind::Bool),
        ("dynamic_startup_nodes", CoercionKind::Bool),
    ];
    match target {
        Target::Cluster => CLUSTER,
        Target::Direct | Target::Url | Target::Sentinel => CLIENT,
    }
}
