use std::collections::HashSet;
use std::mem;
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;
use tokio::sync::Notify;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ToolDiscoveryError {
    #[error("the tool discovery buffer needs room for at least one tool")]
    ZeroCapacity,
}

// ToolDiscoveryQueueItem of litellm/proxy/_types.py; every field is optional there and absent reads like null.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ToolDiscoveryItem {
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub key_hash: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

// ToolDiscoveryQueue: the first item for each tool_name in one flush cycle is kept and the seen set resets on flush. The pending list is bounded by the spend queue's capacity, so a new tool waits for the next flush instead of growing the buffer.
pub struct ToolDiscoveryQueue {
    state: Mutex<PendingTools>,
    room: Notify,
    capacity: usize,
}

#[derive(Default)]
struct PendingTools {
    seen: HashSet<String>,
    pending: Vec<ToolDiscoveryItem>,
}

impl ToolDiscoveryQueue {
    pub fn new(capacity: i64) -> Result<Self, ToolDiscoveryError> {
        let capacity = match usize::try_from(capacity) {
            Ok(capacity) if capacity > 0 => capacity,
            Ok(_empty) | Err(_range) => return Err(ToolDiscoveryError::ZeroCapacity),
        };
        Ok(Self {
            state: Mutex::new(PendingTools::default()),
            room: Notify::new(),
            capacity,
        })
    }

    // add_update: an empty name is no tool, a name already queued this cycle is skipped, and the first item keeps every field of that occurrence.
    pub async fn add_update(&self, item: ToolDiscoveryItem) {
        let Some(tool_name) = item.tool_name.as_deref().filter(|name| !name.is_empty()) else {
            return;
        };
        let tool_name = tool_name.to_owned();
        loop {
            let room = self.room.notified();
            tokio::pin!(room);
            // register before the check, so a flush that frees a slot right after cannot miss this waiter
            room.as_mut().enable();
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.seen.contains(&tool_name) {
                return;
            }
            if state.pending.len() < self.capacity {
                state.seen.insert(tool_name);
                state.pending.push(item);
                return;
            }
            drop(state);
            room.await;
        }
    }

    // flush: hand over the pending items and clear the seen set, so the next cycle counts the same tools again.
    #[must_use]
    pub fn flush(&self) -> Vec<ToolDiscoveryItem> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = mem::take(&mut state.pending);
        state.seen.clear();
        drop(state);
        if !pending.is_empty() {
            self.room.notify_waiters();
        }
        pending
    }
}
