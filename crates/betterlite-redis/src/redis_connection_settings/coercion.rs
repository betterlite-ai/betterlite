mod tests;

use std::collections::BTreeMap;
use std::iter::once;

use super::model::scalars::{BoolOrText, CoercionKind, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::{ClientKind, Target};

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
        IntOrText::Int(parsed) => Some(parsed),
        IntOrText::Text(text) => strip_grouping_underscores(text.trim())?.parse().ok(),
    }
}

#[must_use]
pub fn coerce_float(value: Option<FloatOrText>) -> Option<f64> {
    match value? {
        FloatOrText::Float(parsed) => Some(parsed),
        FloatOrText::Text(text) => strip_grouping_underscores(text.trim())?.parse().ok(),
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
        CoercionKind::Int => strip_grouping_underscores(text.trim())?
            .parse::<i64>()
            .ok()
            .map(|parsed| PassthroughValue::Number(serde_json::Number::from(parsed))),
        CoercionKind::Float => serde_json::Number::from_f64(
            strip_grouping_underscores(text.trim())?
                .parse::<f64>()
                .ok()?,
        )
        .map(PassthroughValue::Number),
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
        ("socket_read_size", CoercionKind::Int),
        ("legacy_responses", CoercionKind::Bool),
        ("ssl_check_hostname", CoercionKind::Bool),
        ("ssl_validate_ocsp", CoercionKind::Bool),
        ("ssl_validate_ocsp_stapled", CoercionKind::Bool),
    ];
    // litellm's coercion_client is (redis.Redis, redis.RedisCluster, async_redis.RedisCluster) for a Cluster target, so Cluster coercion unions Redis.__init__'s signature (CLIENT) with both RedisCluster constructors' signature instead of using RedisCluster's alone.
    const CLUSTER: &[(&str, CoercionKind)] = &[
        ("retry_on_timeout", CoercionKind::Bool),
        ("single_connection_client", CoercionKind::Bool),
        ("decode_responses", CoercionKind::Bool),
        ("health_check_interval", CoercionKind::Int),
        ("socket_read_size", CoercionKind::Int),
        ("legacy_responses", CoercionKind::Bool),
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
