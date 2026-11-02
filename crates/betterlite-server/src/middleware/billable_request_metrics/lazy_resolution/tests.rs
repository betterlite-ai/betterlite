#![cfg(test)]

use std::iter::repeat_with;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use super::LazyResolve;

struct Dummy(u32);

#[test]
fn resolves_lazily_and_caches_the_result() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let resolver: LazyResolve<Dummy> = LazyResolve::new(Some(Box::new(move || {
        factory_calls.fetch_add(1, Ordering::SeqCst);
        Some(Arc::new(Dummy(42)))
    })));

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let first = resolver.get();
    assert_eq!(first.map(|dummy| dummy.0), Some(42));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    resolver.get();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn caches_a_none_result_from_the_factory() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let resolver: LazyResolve<Dummy> = LazyResolve::new(Some(Box::new(move || {
        factory_calls.fetch_add(1, Ordering::SeqCst);
        None
    })));

    assert!(resolver.get().is_none());
    assert!(resolver.get().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn no_factory_always_resolves_to_none() {
    let resolver: LazyResolve<Dummy> = LazyResolve::new(None);
    assert!(resolver.get().is_none());
}

const CONCURRENT_FIRST_REQUESTS: usize = 16;

#[test]
fn concurrent_first_requests_build_only_one_instance() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let resolver = LazyResolve::<Dummy>::new(Some(Box::new(move || {
        factory_calls.fetch_add(1, Ordering::SeqCst);
        thread::yield_now();
        Some(Arc::new(Dummy(7)))
    })));

    thread::scope(|scope| {
        let handles: Vec<_> = repeat_with(|| scope.spawn(|| resolver.get().map(|dummy| dummy.0)))
            .take(CONCURRENT_FIRST_REQUESTS)
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(results.iter().all(|result| *result == Some(7)));
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
