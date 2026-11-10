use std::sync::{Arc, Mutex, Weak};

use super::batches::RequestRedisBatches;

// litellm.caching.redis_batch._open_post_call: requests with declared post-call writes, held weakly so a finished request leaves the registry by itself.
static OPEN_POST_CALL: Mutex<Vec<Weak<RequestRedisBatches>>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
pub(super) enum Change {
    Register,
    Discard,
}

pub(super) fn apply(request: &Arc<RequestRedisBatches>, change: Change) {
    let mut open = match OPEN_POST_CALL.lock() {
        Ok(open) => open,
        Err(poisoned) => poisoned.into_inner(),
    };
    let held = Arc::downgrade(request);
    match change {
        Change::Register => {
            if !open.iter().any(|weak| weak.ptr_eq(&held)) {
                open.push(held);
            }
        }
        Change::Discard => open.retain(|weak| !weak.ptr_eq(&held)),
    }
}

pub async fn drain_post_call_redis_batches() {
    let open: Vec<Arc<RequestRedisBatches>> = {
        let mut open = match OPEN_POST_CALL.lock() {
            Ok(open) => open,
            Err(poisoned) => poisoned.into_inner(),
        };
        open.retain(|held| held.strong_count() > 0);
        open.iter().filter_map(Weak::upgrade).collect()
    };
    for request in &open {
        request.flush_post_call().await;
        request.drain_follow_ups().await;
    }
}
