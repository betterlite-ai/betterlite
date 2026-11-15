use std::collections::HashSet;
use std::time::Duration;

use betterlite_prompts::arguments::json::Json;

use super::hash::display_hash;
use super::store::{RecoveryStore, StoreKey};
use super::tools::RetrieveCall;

pub const MAX_RETRIEVALS_PER_LOOP: usize = 8;

const LIMIT_NOTICE: &str = "[compresr: retrieval limit reached for this turn]";
const REPEAT_NOTICE: &str = "[compresr: already retrieved above for this hash]";

#[derive(Debug, Clone, PartialEq)]
pub struct Retrieved {
    pub call: RetrieveCall,
    pub content: String,
}

#[derive(Debug)]
pub struct Resolution {
    pub retrieved: Vec<Retrieved>,
    pub resolved_any: bool,
}

// Every call gets an answer, a notice when its hash is missing, expired or out of scope, so no tool call lacks a result
#[must_use]
pub fn resolve_retrievals(
    store: &RecoveryStore,
    key: Option<&StoreKey>,
    calls: Vec<RetrieveCall>,
    now: Duration,
) -> Resolution {
    let mut retrieved = Vec::with_capacity(calls.len());
    let mut seen: HashSet<String> = HashSet::new();
    let mut resolved_any = false;
    for (index, call) in calls.into_iter().enumerate() {
        let hash = call
            .arguments
            .get("hash")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        let content = if index >= MAX_RETRIEVALS_PER_LOOP {
            LIMIT_NOTICE.to_owned()
        } else if seen.contains(&hash) {
            REPEAT_NOTICE.to_owned()
        } else if let Some(original) = store.retrieve(key, &hash, now) {
            seen.insert(hash.clone());
            resolved_any = true;
            original
        } else {
            format!(
                "[compresr: hash={} not found, expired, or not issued for this request]",
                display_hash(&hash)
            )
        };
        tracing::debug!(
            hash = %display_hash(&hash),
            chars = content.chars().count(),
            "Compresr retrieve"
        );
        retrieved.push(Retrieved { call, content });
    }
    Resolution {
        retrieved,
        resolved_any,
    }
}
