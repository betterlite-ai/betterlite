#![expect(
    clippy::restriction,
    clippy::pedantic,
    clippy::style,
    clippy::complexity,
    clippy::nursery,
    reason = "Cache-plan JSON is intentionally validation-aware and preserves provider insertion order for fingerprints"
)]

#[path = "cache_plan_planning.rs"]
mod cache_plan_planning;
#[path = "cache_plan_validation.rs"]
mod cache_plan_validation;
use cache_plan_planning::{
    cacheable, chained_hashes, digest, entry_fingerprint, lookbacks, position_groups, prefix_body,
    selected_settings,
};
use cache_plan_validation::{cache_ttl, valid_shape};
use serde_json::{Map, Value};
mod tests;
pub type JsonObject = Map<String, Value>;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheBoundary {
    pub fingerprint: String,
    pub prefix_body: JsonObject,
    pub ttl_seconds: u32,
    pub lookback_fingerprints: Vec<String>,
    pub content_fingerprint: String,
    pub lookback_content_fingerprints: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptCachePlan {
    pub full_body: JsonObject,
    pub breakpoints: Vec<CacheBoundary>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedCacheReason {
    UnsupportedPromptShape,
    ConflictingCacheTtl,
    TooManyCacheBreakpoints,
    InvalidCacheTtlOrder,
    UnsupportedThinkingCache,
    TokenCountUnavailable,
    InconsistentPrefixTokenCount,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedCachePlan {
    pub reason: UnsupportedCacheReason,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptPrefix {
    pub prefix_body: JsonObject,
    pub fingerprint: String,
    pub fingerprints: Vec<String>,
    pub ttl_seconds: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Section {
    Tools,
    System,
    Messages,
}
#[derive(Clone, Debug)]
struct Position {
    section: Section,
    message_index: i64,
    role: String,
    block: JsonObject,
    marker: Option<u32>,
}
pub fn parse_cache_plan(body: &JsonObject) -> Result<PromptCachePlan, UnsupportedCachePlan> {
    if !valid_shape(body, true) {
        return Err(unsupported(UnsupportedCacheReason::UnsupportedPromptShape));
    }
    let mut positions: Vec<Position> = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .map(|tool| cache_plan_planning::make_position(Section::Tools, -1, "", tool))
                .collect()
        })
        .unwrap_or_default();
    if let Some(system) = body.get("system") {
        positions.extend(
            cache_plan_planning::blocks(system)
                .into_iter()
                .map(|block| cache_plan_planning::make_position(Section::System, -1, "", block)),
        );
    }
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for (index, value) in messages.iter().enumerate() {
            let Some(message) = value.as_object() else {
                continue;
            };
            let Some(content) = message.get("content") else {
                continue;
            };
            for block in cache_plan_planning::blocks(content) {
                positions.push(cache_plan_planning::make_position(
                    Section::Messages,
                    i64::try_from(index).unwrap_or(i64::MAX),
                    message.get("role").and_then(Value::as_str).unwrap_or(""),
                    block,
                ));
            }
        }
    }
    let markers = markers(body, &positions)?;
    let hashes = chained_hashes(&positions, body, &selected_settings(body));
    let groups = position_groups(&positions);
    let mut breakpoints = Vec::new();
    for (index, ttl) in markers {
        let Some(hash) = hashes.get(index) else {
            return Err(unsupported(UnsupportedCacheReason::UnsupportedPromptShape));
        };
        breakpoints.push(CacheBoundary {
            fingerprint: entry_fingerprint(hash, ttl),
            prefix_body: prefix_body(body, &positions, index),
            ttl_seconds: ttl,
            lookback_fingerprints: lookbacks(&groups, &hashes, index, ttl, true),
            content_fingerprint: hash.clone(),
            lookback_content_fingerprints: lookbacks(&groups, &hashes, index, ttl, false),
        });
    }
    Ok(PromptCachePlan {
        full_body: body.clone(),
        breakpoints,
    })
}
fn markers(
    body: &JsonObject,
    positions: &[Position],
) -> Result<Vec<(usize, u32)>, UnsupportedCachePlan> {
    let automatic = positions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| cacheable(item));
    let mut result: Vec<(usize, u32)> = positions
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.marker.map(|ttl| (index, ttl)))
        .collect();
    if let Some(request_ttl) = body.get("cache_control").and_then(cache_ttl) {
        if let Some((index, item)) = automatic {
            if let Some(position_ttl) = item.marker {
                if position_ttl != request_ttl {
                    return Err(unsupported(UnsupportedCacheReason::ConflictingCacheTtl));
                }
            } else {
                result.push((index, request_ttl));
            }
        }
    }
    result.sort_by_key(|item| item.0);
    if result.len() > 4 {
        return Err(unsupported(UnsupportedCacheReason::TooManyCacheBreakpoints));
    }
    if result.windows(2).any(|pair| {
        pair.first().map(|item| item.1).unwrap_or(0) < pair.last().map(|item| item.1).unwrap_or(0)
    }) {
        return Err(unsupported(UnsupportedCacheReason::InvalidCacheTtlOrder));
    }
    Ok(result)
}
#[must_use]
pub fn parse_prompt(body: &JsonObject) -> Option<PromptPrefix> {
    if !valid_shape(body, false) {
        return None;
    }
    let plan = parse_cache_plan(body).ok()?;
    if plan.breakpoints.len() != 1 {
        return None;
    }
    let boundary = plan.breakpoints.first()?;
    Some(PromptPrefix {
        prefix_body: boundary.prefix_body.clone(),
        fingerprint: boundary.fingerprint.clone(),
        fingerprints: boundary.lookback_fingerprints.clone(),
        ttl_seconds: boundary.ttl_seconds,
    })
}
#[must_use]
pub fn cache_scope(
    caller_key_hash: &str,
    deployment_id: &str,
    provider_key: &str,
    model: &str,
    anthropic_version: &str,
) -> String {
    let scope = digest(&Value::Array(
        [
            caller_key_hash,
            deployment_id,
            provider_key,
            model,
            anthropic_version,
        ]
        .into_iter()
        .map(|item| Value::String(item.to_owned()))
        .collect(),
    ));
    scope
}
const fn unsupported(reason: UnsupportedCacheReason) -> UnsupportedCachePlan {
    let plan = UnsupportedCachePlan { reason };
    plan
}
