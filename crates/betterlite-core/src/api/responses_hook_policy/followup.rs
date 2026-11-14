use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::AgenticFollowupKwargsError;

pub struct AgenticFollowupInput<'a> {
    pub request_kwargs: &'a IndexMap<String, Json>,
    pub patch_kwargs: &'a IndexMap<String, Json>,
    pub request_params: &'a BTreeSet<String>,
    pub depth: i64,
    pub max_loops: u32,
    pub fingerprints: &'a [Json],
    pub fingerprint: &'a str,
}

// request kwargs first, the patch over them, then the trusted bookkeeping last; a request param is
// never repeated, and an existing key keeps the position it first had
pub fn build_agentic_followup_kwargs(
    input: &AgenticFollowupInput<'_>,
) -> Result<IndexMap<String, Json>, AgenticFollowupKwargsError> {
    let mut kwargs = IndexMap::new();
    for (key, value) in input.request_kwargs {
        if !input.request_params.contains(key) {
            kwargs.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in input.patch_kwargs {
        if !input.request_params.contains(key) {
            kwargs.insert(key.clone(), value.clone());
        }
    }
    let next_depth = input
        .depth
        .checked_add(1)
        .ok_or(AgenticFollowupKwargsError::DepthOverflow)?;
    kwargs.insert(
        "_agentic_loop_depth".to_owned(),
        Json::Number(Number::from(next_depth)),
    );
    kwargs.insert(
        "max_agentic_loops".to_owned(),
        Json::Number(Number::from(input.max_loops)),
    );
    let mut seen = input.fingerprints.to_vec();
    seen.push(Json::Text(input.fingerprint.to_owned()));
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), Json::List(seen));
    Ok(kwargs)
}
