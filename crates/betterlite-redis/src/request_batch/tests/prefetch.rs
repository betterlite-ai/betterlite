use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::scope::request_redis_batch_scope;

#[tokio::test]
async fn request_batch_prefetch_slots_start_empty_and_take_once() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(!request.prefetch_armed("routing_read"));
    assert!(request.take_prefetch("routing_read").is_none());
    assert!(
        request
            .arm_prefetch("routing_read", Box::new("early".to_owned()))
            .is_none()
    );
    assert!(request.prefetch_armed("routing_read"));

    let taken = request
        .take_prefetch("routing_read")
        .expect("the armed slot");
    let text = taken.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("early"));
    assert!(!request.prefetch_armed("routing_read"));
    assert!(request.take_prefetch("routing_read").is_none());
}

#[tokio::test]
async fn request_batch_prefetch_slots_are_named_and_overwritten() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(
        request
            .arm_prefetch("routing_read", Box::new("first".to_owned()))
            .is_none()
    );
    let replaced = request
        .arm_prefetch("routing_read", Box::new("second".to_owned()))
        .expect("the replaced slot");
    let text = replaced.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("first"));
    assert!(
        request
            .arm_prefetch("update_cache_read", Box::new(7_i64))
            .is_none()
    );
    assert!(request.prefetch_armed("routing_read"));
    assert!(request.prefetch_armed("update_cache_read"));

    let other = request
        .take_prefetch("update_cache_read")
        .expect("the other slot");
    assert_eq!(other.downcast::<i64>().ok().map(|value| *value), Some(7));
    assert!(
        request.prefetch_armed("routing_read"),
        "one slot survives another's take"
    );
}
