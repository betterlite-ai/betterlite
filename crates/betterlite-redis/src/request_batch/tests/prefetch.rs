use crate::request_batch::batches::{
    POST_CALL_FLUSH_DEADLINE, PrefetchAction, PrefetchOutcome, RequestRedisBatches,
};
use crate::request_batch::prefetch::PrefetchSlot;
use crate::request_batch::scope::request_redis_batch_scope;

fn is_armed(request: &RequestRedisBatches, slot: &str) -> bool {
    matches!(
        request.prefetch(slot, PrefetchAction::IsArmed),
        PrefetchOutcome::Armed(true)
    )
}

fn take(request: &RequestRedisBatches, slot: &str) -> Option<PrefetchSlot> {
    match request.prefetch(slot, PrefetchAction::Take) {
        PrefetchOutcome::Taken(taken) => taken,
        PrefetchOutcome::Replaced(_) | PrefetchOutcome::Armed(_) => None,
    }
}

fn arm(request: &RequestRedisBatches, slot: &str, value: PrefetchSlot) -> Option<PrefetchSlot> {
    match request.prefetch(slot, PrefetchAction::Arm(value)) {
        PrefetchOutcome::Replaced(replaced) => replaced,
        PrefetchOutcome::Taken(_) | PrefetchOutcome::Armed(_) => None,
    }
}

#[tokio::test]
async fn request_batch_prefetch_slots_start_empty_and_take_once() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(!is_armed(&request, "routing_read"));
    assert!(take(&request, "routing_read").is_none());
    assert!(arm(&request, "routing_read", Box::new("early".to_owned())).is_none());
    assert!(is_armed(&request, "routing_read"));

    let taken = take(&request, "routing_read").expect("the armed slot");
    let text = taken.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("early"));
    assert!(!is_armed(&request, "routing_read"));
    assert!(take(&request, "routing_read").is_none());
}

#[tokio::test]
async fn request_batch_prefetch_slots_are_named_and_overwritten() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(arm(&request, "routing_read", Box::new("first".to_owned())).is_none());
    let replaced =
        arm(&request, "routing_read", Box::new("second".to_owned())).expect("the replaced slot");
    let text = replaced.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("first"));
    assert!(arm(&request, "update_cache_read", Box::new(7_i64)).is_none());
    assert!(is_armed(&request, "routing_read"));
    assert!(is_armed(&request, "update_cache_read"));

    let other = take(&request, "update_cache_read").expect("the other slot");
    assert_eq!(other.downcast::<i64>().ok().map(|value| *value), Some(7));
    assert!(is_armed(&request, "routing_read"));
}
