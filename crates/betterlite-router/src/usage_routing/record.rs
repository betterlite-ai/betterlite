use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::router_predicates::JsonMap;
use crate::router_predicates::source_text::source_text;
use crate::router_predicates::success_key::success_minute_key;
use crate::usage_routing::window::{UsageWindows, window_keys};

// One successful response as the success callbacks read it: the deployment, its tokens and the call type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuccessUse<'use_> {
    pub call_type: Option<&'use_ str>,
    pub litellm_params: &'use_ JsonMap,
    pub total_tokens: Option<i64>,
}

// The read-modify-write both success callbacks run; docs/deviations/B317.yaml records the store-failure answers.
pub fn record_success(
    windows: &dyn UsageWindows,
    ttl_seconds: i64,
    now: DateTime<Utc>,
    success: &SuccessUse<'_>,
) {
    let Some(total_tokens) = success.total_tokens else {
        return;
    };
    let Some(deployment_id) =
        success_minute_key(success.call_type, success.litellm_params, str::to_owned)
    else {
        return;
    };
    let Some(group) = model_group(success.litellm_params) else {
        return;
    };
    let (tokens_key, requests_key) = window_keys(&group, now);
    for (key, weight) in [(tokens_key, total_tokens), (requests_key, 1)] {
        let stored = match windows.read(&key) {
            Ok(stored) => stored,
            Err(error) => {
                tracing::warn!(%error, %key, "the usage window update was dropped");
                continue;
            }
        };
        let mut counts = stored.unwrap_or_default();
        let next = counts
            .get(&deployment_id)
            .copied()
            .unwrap_or_default()
            .saturating_add(weight);
        counts.insert(deployment_id.clone(), next);
        if let Err(error) = windows.write(&key, &counts, ttl_seconds) {
            tracing::warn!(%error, %key, "the usage window update was dropped");
        }
    }
}

// success_minute_key already read metadata.model_group as present and non-null, so this only takes its text.
fn model_group(litellm_params: &JsonMap) -> Option<String> {
    let Some(Json::Object(metadata)) = litellm_params.get("metadata") else {
        return None;
    };
    metadata.get("model_group").map(source_text)
}
