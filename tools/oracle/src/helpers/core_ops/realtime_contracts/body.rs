use super::failure;
use crate::helpers::HelperResult;
use betterlite_core::api::realtime_contracts::cache::RealtimeRequestBodyCache;
use betterlite_core::api::realtime_contracts::decode::{
    RealtimeDecodeLimits, decode_realtime_frame,
};
use serde::Deserialize;
use serde_json::json;
const LIMITS: RealtimeDecodeLimits = RealtimeDecodeLimits { max_bytes: 1 << 20 };
const DISTINCT_MODELS: usize = 1001;
#[derive(Debug, Deserialize)]
pub(crate) struct CachedBodyInput {
    models: Vec<Option<String>>,
    payloads: Vec<String>,
}

pub(super) fn cached_body_and_frames(input: &CachedBodyInput) -> HelperResult {
    let cache = RealtimeRequestBodyCache::new();
    let bodies = input
        .models
        .iter()
        .map(|model| String::from_utf8_lossy(&cache.body(model.as_deref())).into_owned())
        .collect::<Vec<_>>();
    (0..DISTINCT_MODELS).for_each(|index| drop(cache.body(Some(&format!("model-{index}")))));
    let size = cache.occupancy().len;
    let repeat = String::from_utf8_lossy(&cache.body(Some("model-0"))).into_owned();
    let frames = input
        .payloads
        .iter()
        .map(|payload| {
            let frame = decode_realtime_frame(payload.as_bytes(), LIMITS)
                .map_err(|error| failure(error.to_string()))?;
            serde_json::to_value(frame).map_err(|error| failure(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "capacity": cache.occupancy().capacity,
        "bodies": bodies,
        "size_after_distinct": size,
        "repeat_body": repeat,
        "frames": frames,
    }))
}
