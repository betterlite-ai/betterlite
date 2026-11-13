use std::sync::{Arc, Mutex};

use betterlite_context::service_trace_id_cache::ServiceTraceIDCache;

type TestTime = Arc<Mutex<f64>>;

fn fake_clock(time: &TestTime) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    let shared = Arc::clone(time);
    Arc::new(move || *shared.lock().expect("fixture clock lock is not poisoned"))
}

fn advance(time: &TestTime, seconds: f64) {
    let mut now = time.lock().expect("fixture clock lock is not poisoned");
    *now = seconds.mul_add(1.0_f64, *now);
}

fn cache_at(time: &TestTime) -> ServiceTraceIDCache {
    ServiceTraceIDCache::new(fake_clock(time)).expect("the fixture cache is built")
}

#[test]
fn service_trace_id_cache_returns_the_trace_id_for_its_service_and_call() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);

    cache
        .set_cache("call-1", "langfuse", "trace-a")
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache("call-1", "langfuse").expect("get succeeds"),
        Some("trace-a".to_owned())
    );
}

#[test]
fn trace_id_cache_separates_services_and_calls() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);

    cache
        .set_cache("call-1", "langfuse", "trace-a")
        .expect("set succeeds");
    cache
        .set_cache("call-2", "langfuse", "trace-b")
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache("call-1", "datadog").expect("get succeeds"),
        None
    );
    assert_eq!(
        cache.get_cache("call-2", "langfuse").expect("get succeeds"),
        Some("trace-b".to_owned())
    );
}

#[test]
fn service_trace_id_cache_returns_none_for_an_unknown_call() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);

    assert_eq!(
        cache
            .get_cache("missing", "langfuse")
            .expect("get succeeds"),
        None
    );
}

#[test]
fn service_trace_id_cache_expires_after_the_default_ttl() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);
    cache
        .set_cache("call-1", "langfuse", "trace-a")
        .expect("set succeeds");

    advance(&time, 599.5_f64);
    assert_eq!(
        cache.get_cache("call-1", "langfuse").expect("get succeeds"),
        Some("trace-a".to_owned())
    );

    advance(&time, 0.5_f64);
    assert_eq!(
        cache.get_cache("call-1", "langfuse").expect("get succeeds"),
        Some("trace-a".to_owned())
    );

    advance(&time, 0.5_f64);
    assert_eq!(
        cache.get_cache("call-1", "langfuse").expect("get succeeds"),
        None
    );
}

#[test]
fn trace_id_cache_evicts_the_earliest_entry_after_two_hundred_entries() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);
    (0..200_u32).for_each(|index| {
        cache
            .set_cache(&format!("call-{index:03}"), "langfuse", "trace")
            .expect("set succeeds");
    });

    assert!(
        cache
            .get_cache("call-000", "langfuse")
            .expect("get succeeds")
            .is_some()
    );

    cache
        .set_cache("call-200", "langfuse", "trace")
        .expect("set succeeds");

    assert_eq!(
        cache
            .get_cache("call-000", "langfuse")
            .expect("get succeeds"),
        None
    );
    assert!(
        cache
            .get_cache("call-001", "langfuse")
            .expect("get succeeds")
            .is_some()
    );
    assert!(
        cache
            .get_cache("call-200", "langfuse")
            .expect("get succeeds")
            .is_some()
    );
}

#[test]
fn service_trace_id_cache_returns_a_numeric_looking_trace_id_unchanged() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = cache_at(&time);

    cache
        .set_cache("call-1", "langfuse", "12345678901234567890")
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache("call-1", "langfuse").expect("get succeeds"),
        Some("12345678901234567890".to_owned())
    );
}
