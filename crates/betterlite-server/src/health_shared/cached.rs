use std::collections::BTreeMap;
use std::ops::{Add as _, Div as _};

use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{
    DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::cache_primitives::decode::CachedValue;
use chrono::{DateTime, Utc};
use num_traits::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::health_cycle::endpoints::HealthCheckResult;

use super::error::CachedHealthError;

const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

// safe_dumps' dict as cache_health_check_results writes it; the counts are written but never read back.
#[derive(Serialize)]
struct CachedWrite<'write> {
    healthy_endpoints: &'write [JsonObject],
    unhealthy_endpoints: &'write [JsonObject],
    healthy_count: usize,
    unhealthy_count: usize,
    timestamp: f64,
    checked_by: &'write str,
}

// A cached result as the reader needs it; a missing timestamp reads as 0, so it expires at once.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CachedHealthCheck {
    #[serde(default)]
    pub healthy_endpoints: Vec<JsonObject>,
    #[serde(default)]
    pub unhealthy_endpoints: Vec<JsonObject>,
    pub timestamp: Option<Number>,
    #[serde(default)]
    pub checked_by: Option<String>,
}

impl CachedHealthCheck {
    pub fn timestamp_seconds(&self) -> f64 {
        self.timestamp
            .as_ref()
            .and_then(Number::as_f64)
            .unwrap_or_default()
    }
}

// cache_health_check_results' value: safe_dumps over the four fields and the pod that ran the check.
pub fn encode_cached(
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
    timestamp: f64,
    checked_by: &str,
) -> Result<String, PrivateJsonError> {
    let payload = CachedWrite {
        healthy_endpoints: healthy,
        unhealthy_endpoints: unhealthy,
        healthy_count: healthy.len(),
        unhealthy_count: unhealthy.len(),
        timestamp,
        checked_by,
    };
    safe_dumps(&payload, DEFAULT_MAX_RECURSE_DEPTH, None)
}

// isinstance(cached_data, str) ? json.loads(...) : cached_data, then the payload's own row shape.
pub fn cached_payload(value: CachedValue) -> Result<CachedHealthCheck, CachedHealthError> {
    let cached = json_from_cached(value);
    let document = if let Json::Text(text) = &cached {
        safe_json_loads(text.as_bytes())?
    } else {
        serde_json::to_value(&cached)?
    };
    Ok(serde_json::from_value(document)?)
}

// betterlite-redis keeps its CachedValue -> Json mapping crate-private, so this reader maps it here.
#[must_use]
pub fn json_from_cached(value: CachedValue) -> Json {
    match value {
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => Json::Bool(flag),
        CachedValue::Number(number) => Json::Number(number),
        CachedValue::Text(text) => Json::Text(text),
        CachedValue::Array(items) => Json::List(items.into_iter().map(json_from_cached).collect()),
        CachedValue::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, entry)| (key, json_from_cached(entry)))
                .collect(),
        ),
    }
}

// time.time() as the cache stores it: whole seconds plus the fractional part.
#[must_use]
pub fn epoch_seconds(now: DateTime<Utc>) -> f64 {
    let seconds = now.timestamp().to_f64().unwrap_or_default();
    let nanos = now.timestamp_subsec_nanos().to_f64().unwrap_or_default();
    seconds.add(nanos.div(NANOS_PER_SECOND))
}

// The lock's stored value is the pod id as a JSON string, the only form that can name an owner.
pub(crate) fn cached_owner(value: CachedValue) -> Option<String> {
    match value {
        CachedValue::Text(text) => Some(text),
        CachedValue::Null
        | CachedValue::Boolean(_)
        | CachedValue::Number(_)
        | CachedValue::Array(_)
        | CachedValue::Object(_) => None,
    }
}

// A cache hit keeps the rows and loses the exceptions, which only the running pod holds live.
pub(crate) fn from_cached(cached: CachedHealthCheck) -> HealthCheckResult {
    HealthCheckResult {
        healthy_endpoints: cached.healthy_endpoints,
        unhealthy_endpoints: cached.unhealthy_endpoints,
        exceptions_by_model_id: BTreeMap::new(),
    }
}
