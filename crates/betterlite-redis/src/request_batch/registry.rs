use std::sync::{Arc, Mutex, Weak};

use futures_util::future::join_all;

use super::batches::RequestRedisBatches;
use super::lock;

// litellm.caching.redis_batch._open_post_call: requests with declared post-call writes, held weakly so a finished request leaves the registry by itself.
pub(super) static OPEN_POST_CALL: Mutex<Vec<Weak<RequestRedisBatches>>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
pub(super) enum Change {
    Register,
    Discard,
}

pub(super) fn apply(request: &Arc<RequestRedisBatches>, change: Change) {
    let mut open = lock(&OPEN_POST_CALL);
    open.retain(|held| held.strong_count() > 0);
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
        let mut open = lock(&OPEN_POST_CALL);
        open.retain(|held| held.strong_count() > 0);
        open.iter().filter_map(Weak::upgrade).collect()
    };
    join_all(open.iter().map(RequestRedisBatches::flush_post_call)).await;
    join_all(open.iter().map(|request| request.drain_follow_ups())).await;
}
