#![cfg(test)]

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::tests::support::fixture;
use crate::prometheus_hooks::PrometheusHooks;

#[test]
fn shared_builds_one_instance_for_every_call() {
    let builds = AtomicUsize::new(0);
    let first = PrometheusHooks::shared(|| {
        builds.fetch_add(1, Ordering::SeqCst);
        Ok::<PrometheusHooks, Infallible>(fixture().hooks)
    })
    .unwrap();
    let second = PrometheusHooks::shared(|| -> Result<PrometheusHooks, Infallible> {
        builds.fetch_add(1, Ordering::SeqCst);
        panic!("the build closure ran a second time")
    })
    .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&first, &second));
}
