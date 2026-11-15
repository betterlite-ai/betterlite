use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog_llm_obs::values::mapping_field;

use super::presentation::rounded_to_two;

pub const DEPLOYMENT_LATENCIES_KEY: &str = "_latency_per_deployment";

#[must_use]
pub fn deployment_latencies(metadata: &IndexMap<String, Json>) -> Option<String> {
    let latencies = mapping_field(metadata, DEPLOYMENT_LATENCIES_KEY)?;
    if latencies.is_empty() {
        return None;
    }
    let mut pairs = Vec::with_capacity(latencies.len());
    for (api_base, value) in latencies {
        let latency = match value {
            Json::Number(number) => number.as_f64()?,
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
                return None;
            }
        };
        pairs.push((api_base.as_str(), latency));
    }
    pairs.sort_by(|left, right| left.1.total_cmp(&right.1));
    let mut message = String::from("```");
    for (api_base, latency) in pairs {
        let row = format!("\n{api_base}: {}s", rounded_to_two(latency));
        message.push_str(&row);
    }
    message.push_str("```");
    Some(message)
}
