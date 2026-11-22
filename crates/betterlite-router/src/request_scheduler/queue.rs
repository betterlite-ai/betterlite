use super::error::SchedulerError;
use crate::routing_value_store::{RouteKey, RouteValue, SchedulerTuple, StoreRead};
use std::collections::BTreeSet;
// SchedulerCacheKeys.queue: one cache entry per model group
const PREFIX: &str = "scheduler:queue:";
const SEPARATOR: char = ':';
// the queue bound a request past it is refused with
pub const MAX_PENDING_REQUESTS: usize = 1024;
// a uuid4 id is 36 bytes, and the bound keeps an encoded member well inside the store's value limit
pub const MAX_REQUEST_ID_BYTES: usize = 128;
// Scheduler.get_queue's `scheduler:queue:{model_name}` key
pub fn key(model: &str) -> Result<RouteKey, SchedulerError> {
    match RouteKey::parse(&format!("{PREFIX}{model}")) {
        Ok(key) => Ok(key),
        Err(source) => Err(SchedulerError::Key { source }),
    }
}
// the members a queue write stores; three digits make string order agree with the tuple order
pub fn encoded(entries: &[SchedulerTuple]) -> BTreeSet<String> {
    let mut members = BTreeSet::new();
    for entry in entries {
        members.insert(format!(
            "{:03}{SEPARATOR}{}",
            entry.priority, entry.request_id
        ));
    }
    members
}
#[must_use]
pub fn decode(member: &str) -> Option<SchedulerTuple> {
    let (priority, request_id) = member.split_once(SEPARATOR)?;
    Some(SchedulerTuple {
        priority: priority.parse::<u8>().ok()?,
        request_id: request_id.to_owned(),
    })
}
// `get_queue` as the queue's full order; the pin's heap holds only its head in order
#[must_use]
pub fn scheduled(members: &BTreeSet<String>) -> Vec<SchedulerTuple> {
    members
        .iter()
        .filter_map(|member| decode(member.as_str()))
        .collect()
}
// `queue[0]`, the tuple a poll or peek compares the request id against
#[must_use]
pub fn head(members: &BTreeSet<String>) -> Option<SchedulerTuple> {
    members
        .iter()
        .next()
        .and_then(|member| decode(member.as_str()))
}
// `get_queue`'s list, or the empty queue a missing entry means
pub fn members(read: &StoreRead, model: &str) -> Result<BTreeSet<String>, SchedulerError> {
    match read {
        StoreRead::Missing => Ok(BTreeSet::new()),
        StoreRead::Value(RouteValue::Members(members)) => Ok(members.clone()),
        StoreRead::Value(
            RouteValue::Counter(_)
            | RouteValue::Spend(_)
            | RouteValue::Pin(_)
            | RouteValue::SchedulerTuple(_),
        ) => Err(SchedulerError::NotAQueue {
            model: model.to_owned(),
        }),
    }
}
// remove_request's list comprehension: every entry whose request id differs survives
#[must_use]
pub fn without(members: &BTreeSet<String>, request_id: &str) -> BTreeSet<String> {
    members
        .iter()
        .filter(|member| match decode(member.as_str()) {
            Some(entry) => entry.request_id != request_id,
            None => true,
        })
        .cloned()
        .collect()
}
// an id must survive encoding and stay inside the store's member bound
#[must_use]
pub fn valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= MAX_REQUEST_ID_BYTES
        && !request_id.contains(SEPARATOR)
}
