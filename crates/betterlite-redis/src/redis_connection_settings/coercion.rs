mod tests;

use std::iter::once;

use super::model::scalars::{BoolOrText, CoercionKind, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::Target;

// Python's int()/float() accept a single underscore between two digits as a grouping separator (PEP 515) and reject every other placement, including doubled underscores such as "6__380". Stripping every underscore unconditionally would instead accept that input.
fn strip_grouping_underscores(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let prevs = once(None).chain(chars.iter().copied().map(Some));
    let nexts = chars.iter().copied().skip(1).map(Some).chain(once(None));
    for ((prev, current), next) in prevs.zip(chars.iter().copied()).zip(nexts) {
        if current != '_' {
            continue;
        }
        let prev_is_digit = prev.is_some_and(|ch| ch.is_ascii_digit());
        let next_is_digit = next.is_some_and(|ch| ch.is_ascii_digit());
        if !prev_is_digit || !next_is_digit {
            return None;
        }
    }
    Some(chars.into_iter().filter(|ch| *ch != '_').collect())
}

#[must_use]
pub fn coerce_int(value: Option<IntOrText>) -> Option<i64> {
    match value? {
        IntOrText::Value(parsed) => Some(parsed),
        IntOrText::Text(text) => strip_grouping_underscores(text.trim())?.parse().ok(),
    }
}

#[must_use]
pub fn coerce_float(value: Option<FloatOrText>) -> Option<f64> {
    match value? {
        FloatOrText::Value(parsed) => Some(parsed),
        FloatOrText::Text(text) => strip_grouping_underscores(text.trim())?.parse().ok(),
    }
}

// _str_to_bool never fails: any string that is not "true"/"1"/"yes" coerces to false.
#[must_use]
pub fn coerce_bool(value: Option<BoolOrText>) -> Option<bool> {
    match value? {
        BoolOrText::Value(parsed) => Some(parsed),
        BoolOrText::Text(text) => {
            Some(matches!(text.to_lowercase().as_str(), "true" | "1" | "yes"))
        }
    }
}

// A failed int()/float() conversion deletes the key outright in litellm; bool never fails.
pub fn coerce_passthrough(name: &str, value: &mut PassthroughValue, target: Target) -> bool {
    let (Some(kind), PassthroughValue::Text(text)) = (coercion_kind(name, target), &*value) else {
        return true;
    };
    coerce_passthrough_value(kind, text).is_some_and(|coerced| {
        *value = coerced;
        true
    })
}

fn coerce_passthrough_value(kind: CoercionKind, text: &str) -> Option<PassthroughValue> {
    match kind {
        CoercionKind::Int => strip_grouping_underscores(text.trim())?
            .parse::<i64>()
            .ok()
            .map(|parsed| PassthroughValue::Number(serde_json::Number::from(parsed))),
        CoercionKind::Bool => Some(PassthroughValue::Bool(matches!(
            text.to_lowercase().as_str(),
            "true" | "1" | "yes"
        ))),
    }
}

fn coercion_kind(name: &str, target: Target) -> Option<CoercionKind> {
    match name {
        "retry_on_timeout"
        | "single_connection_client"
        | "decode_responses"
        | "legacy_responses"
        | "ssl_check_hostname"
        | "ssl_validate_ocsp"
        | "ssl_validate_ocsp_stapled" => Some(CoercionKind::Bool),
        "health_check_interval" | "socket_read_size" => Some(CoercionKind::Int),
        "cluster_error_retry_attempts" | "reinitialize_steps" if target == Target::Cluster => {
            Some(CoercionKind::Int)
        }
        "require_full_coverage" | "read_from_replicas" | "dynamic_startup_nodes"
            if target == Target::Cluster =>
        {
            Some(CoercionKind::Bool)
        }
        _ => None,
    }
}
