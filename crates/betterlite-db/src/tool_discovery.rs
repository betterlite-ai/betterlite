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
    pub fn new(queue_capacity: i64) -> Result<Self, ToolDiscoveryError> {
        let Ok(capacity) = usize::try_from(queue_capacity) else {
            return Err(ToolDiscoveryError::ZeroCapacity);
        };
        if capacity == 0 {
            return Err(ToolDiscoveryError::ZeroCapacity);
        }
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
        let queued_name = tool_name.to_owned();
        let mut waiting = (item, queued_name);
        loop {
            let room = self.room.notified();
            tokio::pin!(room);
            // register before the check, so a flush that frees a slot right after cannot miss this waiter
            room.as_mut().enable();
            let Some(rejected) = self.try_queue(waiting) else {
                return;
            };
            waiting = rejected;
            room.await;
        }
    }

    // A full buffer hands the tool and its name back, so a producer keeps them across the wait for room.
    fn try_queue(
        &self,
        (item, queued_name): (ToolDiscoveryItem, String),
    ) -> Option<(ToolDiscoveryItem, String)> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.seen.contains(&queued_name) {
            return None;
        }
        if state.pending.len() < self.capacity {
            state.seen.insert(queued_name);
            state.pending.push(item);
            return None;
        }
        Some((item, queued_name))
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
