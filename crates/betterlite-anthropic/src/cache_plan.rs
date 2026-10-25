mod breakpoints;
mod decode;
mod json;
mod prefix;
mod tests;
pub mod wire;

use serde::Serialize;
use serde_json::value::RawValue;

use crate::cache_plan::breakpoints::{groups, lookback, markers};
use crate::cache_plan::decode::{Decoded, Section, decode};
use crate::cache_plan::json::digest;
use crate::cache_plan::prefix::prefix_body;
use crate::cache_plan::wire::Ttl;

const ENTRY_SCOPE: &str = "native-cache-prefix-v2";

#[derive(Debug, Clone, Serialize)]
pub struct CacheBoundary {
    pub fingerprint: String,
    pub prefix_body: Box<RawValue>,
    pub ttl_seconds: u32,
    pub lookback_fingerprints: Vec<String>,
    pub content_fingerprint: String,
    pub lookback_content_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptCachePlan {
    pub full_body: Box<RawValue>,
    pub breakpoints: Vec<CacheBoundary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptPrefix {
    pub prefix_body: Box<RawValue>,
    pub fingerprint: String,
    pub fingerprints: Vec<String>,
    pub ttl_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedCachePlan {
    #[error("the request does not have a shape the cache planner models")]
    UnsupportedPromptShape,
    #[error("the request cache_control ttl differs from the marker on the last cacheable block")]
    ConflictingCacheTtl,
    #[error("the request marks more than four cache breakpoints")]
    TooManyCacheBreakpoints,
    #[error("a 1h cache breakpoint follows a 5m one")]
    InvalidCacheTtlOrder,
    #[error("the model's thinking cache semantics are not known")]
    UnsupportedThinkingCacheSemantics,
    #[error("the provider did not count the prompt tokens")]
    TokenCountUnavailable,
    #[error("a longer prefix counted fewer tokens than a shorter one")]
    InconsistentPrefixTokenCount,
}

#[derive(Serialize)]
struct Settings<'plan> {
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<&'plan RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<&'plan RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<&'plan RawValue>,
}

pub fn parse_cache_plan(body: &str) -> Result<PromptCachePlan, UnsupportedCachePlan> {
    let Ok(decoded) = decode(body) else {
        return Err(UnsupportedCachePlan::UnsupportedPromptShape);
    };
    plan(decoded)
}

#[must_use]
pub fn parse_prompt(body: &str) -> Option<PromptPrefix> {
    let decoded = decode(body).ok().filter(|decoded| decoded.predictable)?;
    let [boundary] = <[CacheBoundary; 1]>::try_from(plan(decoded).ok()?.breakpoints).ok()?;
    Some(PromptPrefix {
        prefix_body: boundary.prefix_body,
        fingerprint: boundary.fingerprint,
        fingerprints: boundary.lookback_fingerprints,
        ttl_seconds: boundary.ttl_seconds,
    })
}

pub fn cache_scope(
    caller_key_hash: &str,
    deployment_id: &str,
    provider_key: &str,
    model: &str,
    anthropic_version: &str,
) -> Result<String, serde_json::Error> {
    digest(&(
        caller_key_hash,
        deployment_id,
        provider_key,
        model,
        anthropic_version,
    ))
}

fn plan(decoded: Decoded) -> Result<PromptCachePlan, UnsupportedCachePlan> {
    let markers = markers(&decoded)?;
    let Ok(breakpoints) = breakpoints(&decoded, &markers) else {
        return Err(UnsupportedCachePlan::UnsupportedPromptShape);
    };
    Ok(PromptCachePlan {
        full_body: decoded.full_body,
        breakpoints,
    })
}

fn breakpoints(
    decoded: &Decoded,
    markers: &[Option<Ttl>],
) -> Result<Vec<CacheBoundary>, serde_json::Error> {
    let chain = chain(decoded)?;
    let groups = groups(&decoded.positions);
    let mut breakpoints = Vec::new();
    for (index, (hash, marker)) in chain.iter().zip(markers).enumerate() {
        let Some(ttl) = marker else {
            continue;
        };
        let seconds = ttl.seconds();
        let window = lookback(&groups, &chain, index);
        let mut fingerprints = Vec::with_capacity(window.len());
        for earlier in &window {
            fingerprints.push(digest(&(ENTRY_SCOPE, earlier, seconds))?);
        }
        breakpoints.push(CacheBoundary {
            fingerprint: digest(&(ENTRY_SCOPE, hash, seconds))?,
            prefix_body: prefix_body(decoded, index)?,
            ttl_seconds: seconds,
            lookback_fingerprints: fingerprints,
            content_fingerprint: hash.clone(),
            lookback_content_fingerprints: window.into_iter().map(str::to_owned).collect(),
        });
    }
    Ok(breakpoints)
}

fn chain(decoded: &Decoded) -> Result<Vec<String>, serde_json::Error> {
    let mut previous = digest(&Settings {
        thinking: decoded.thinking.as_deref(),
        output_config: decoded.output_config.as_deref(),
        speed: decoded.speed.as_deref(),
    })?;
    let mut chain = Vec::with_capacity(decoded.positions.len());
    for position in &decoded.positions {
        let tool_choice = match position.section {
            Section::Messages => decoded.tool_choice.as_deref(),
            Section::Tools | Section::System => None,
        };
        let current = digest(&(
            position.section,
            position.message_index,
            &position.role,
            &position.fields,
            tool_choice,
        ))?;
        previous = digest(&(&previous, &current))?;
        chain.push(previous.clone());
    }
    Ok(chain)
}
