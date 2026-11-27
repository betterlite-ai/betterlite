use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::router_compaction::ContextCompactionConfig;
use betterlite_models::decisions::Json;

use crate::compaction_history::blocks::field_text;
use crate::compaction_history::contracts::Surface;
use crate::router_predicates::JsonMap;

mod tests;

const CLIENT_MANAGED_KEYS: [&str; 4] = [
    "previous_response_id",
    "conversation",
    "context_management",
    "compaction",
];

// request kwargs are a typed JSON map that cannot hold the state, so it travels beside them
pub type SharedCompactionState<Summary> = Arc<Mutex<CompactionState<Summary>>>;

// LiteLLM's one _context_compaction_state per routed call: B336 arms it and fills the summary
pub struct CompactionState<Summary> {
    pub config: Option<ContextCompactionConfig>,
    pub candidates: Vec<String>,
    pub summary: Option<(String, Summary)>,
    pub parent_model: Option<String>,
    pub surface: Option<Surface>,
}

// every fallback hop keeps the first attempt's state; a fresh one loses its arming and summary
#[must_use]
pub fn initialize_compaction_state<Summary>(
    existing: Option<&SharedCompactionState<Summary>>,
    surface: Surface,
) -> SharedCompactionState<Summary> {
    let Some(state) = existing else {
        return Arc::new(Mutex::new(CompactionState {
            config: None,
            candidates: Vec::new(),
            summary: None,
            parent_model: None,
            surface: Some(surface),
        }));
    };
    Arc::clone(state)
}

// public because B336's arm_compaction and compact_to_fit ask it too
#[must_use]
pub fn client_managed(payload: &JsonMap) -> bool {
    let top_level = CLIENT_MANAGED_KEYS.iter().any(|key| {
        payload
            .get(*key)
            .is_some_and(|value| !matches!(value, Json::Null))
    });
    top_level || client_managed_input(payload)
}

// only the input list counts: encrypted or thinking blocks in messages still leave a request pending
fn client_managed_input(payload: &JsonMap) -> bool {
    let Some(Json::List(items)) = payload.get("input") else {
        return false;
    };
    items.iter().any(client_managed_item)
}

fn client_managed_item(item: &Json) -> bool {
    let Json::Object(fields) = item else {
        return false;
    };
    if matches!(
        field_text(fields, "type"),
        Some("reasoning" | "compaction" | "item_reference")
    ) {
        return true;
    }
    if fields
        .get("encrypted_content")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return true;
    }
    let Some(Json::List(blocks)) = fields.get("content") else {
        return false;
    };
    blocks.iter().any(encrypted_block)
}

fn encrypted_block(block: &Json) -> bool {
    let Json::Object(fields) = block else {
        return false;
    };
    field_text(fields, "type") == Some("encrypted_content")
}

#[must_use]
pub fn compaction_pending<Summary>(
    state: Option<&SharedCompactionState<Summary>>,
    payload: &JsonMap,
) -> bool {
    let Some(state) = state else {
        return false;
    };
    let armed = state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .config
        .is_some();
    armed && !client_managed(payload)
}
