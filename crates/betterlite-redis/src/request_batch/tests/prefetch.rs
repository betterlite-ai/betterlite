use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::lock;
use crate::request_batch::scope::request_redis_batch_scope;

#[tokio::test]
async fn request_batch_prefetch_slots_start_empty_and_take_once() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(!lock(&request.prefetched).contains_key("routing_read"));
    assert!(lock(&request.prefetched).remove("routing_read").is_none());
    assert!(
        lock(&request.prefetched)
            .insert("routing_read".to_owned(), Box::new("early".to_owned()))
            .is_none()
    );
    assert!(lock(&request.prefetched).contains_key("routing_read"));

    let taken = lock(&request.prefetched)
        .remove("routing_read")
        .expect("the armed slot");
    let text = taken.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("early"));
    assert!(!lock(&request.prefetched).contains_key("routing_read"));
    assert!(lock(&request.prefetched).remove("routing_read").is_none());
}

#[tokio::test]
async fn request_batch_prefetch_slots_are_named_and_overwritten() {
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;

    assert!(
        lock(&request.prefetched)
            .insert("routing_read".to_owned(), Box::new("first".to_owned()))
            .is_none()
    );
    let replaced = lock(&request.prefetched)
        .insert("routing_read".to_owned(), Box::new("second".to_owned()))
        .expect("the replaced slot");
    let text = replaced.downcast::<String>().ok().map(|value| *value);
    assert_eq!(text.as_deref(), Some("first"));
    assert!(
        lock(&request.prefetched)
            .insert("update_cache_read".to_owned(), Box::new(7_i64))
            .is_none()
    );
    assert!(lock(&request.prefetched).contains_key("routing_read"));
    assert!(lock(&request.prefetched).contains_key("update_cache_read"));

    let other = lock(&request.prefetched)
        .remove("update_cache_read")
        .expect("the other slot");
    assert_eq!(other.downcast::<i64>().ok().map(|value| *value), Some(7));
    assert!(lock(&request.prefetched).contains_key("routing_read"));
}
