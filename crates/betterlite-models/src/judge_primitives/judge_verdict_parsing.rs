use fancy_regex::Regex;
use indexmap::IndexMap;

use crate::decisions::Json;
use crate::judge_primitives::error::JudgePrimitivesError;

mod tests;

// LiteLLM's JSON_FENCE_RE: optional ```json marker, the payload, then the closing fence
const JSON_FENCE_PATTERN: &str = r"(?is)```(?:json)?\s*(.*?)\s*```";

pub fn parse_json_verdict(raw: &str) -> Result<IndexMap<String, Json>, JudgePrimitivesError> {
    let payload = fenced_payload(raw)?;
    match serde_json::from_str::<Json>(payload) {
        Ok(Json::Object(verdict)) => Ok(verdict),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(JudgePrimitivesError::NotAnObject)
        }
        Err(first) => braced_payload(payload).map_or_else(
            || Err(JudgePrimitivesError::Json { source: first }),
            |braced| match serde_json::from_str::<Json>(braced) {
                Ok(Json::Object(verdict)) => Ok(verdict),
                Ok(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                ) => Err(JudgePrimitivesError::NotAnObject),
                Err(second) => Err(JudgePrimitivesError::Json { source: second }),
            },
        ),
    }
}

// The first fence wins even when prose follows it, so a later object never replaces the reply
fn fenced_payload(raw: &str) -> Result<&str, JudgePrimitivesError> {
    let text = raw.trim();
    let fence = Regex::new(JSON_FENCE_PATTERN)
        .map_err(|source| JudgePrimitivesError::FencePattern { source })?;
    let captured = fence
        .captures(text)
        .map_err(|source| JudgePrimitivesError::FenceScan { source })?;
    Ok(captured
        .and_then(|captures| captures.get(1))
        .map_or(text, |payload| payload.as_str().trim()))
}

// Python takes find("{") to rfind("}") inclusive and re-raises when the braces do not bracket text
fn braced_payload(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    text.get(start..=end)
}
