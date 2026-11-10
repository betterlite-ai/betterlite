#![cfg(test)]

use rstest::rstest;

use super::{NoRequestScope, RequestOtelScope, RequestScopeSource as _};
use crate::otel::fan_out::tests::support::destination_with;

#[rstest]
#[case::override_mode(false)]
#[case::additive_mode(true)]
fn is_suppressed_reads_the_request_mode(#[case] additive: bool) {
    let request = RequestOtelScope {
        destinations: vec![destination_with("http://collector", &[], Some("arize"))],
        additive,
    };
    assert_eq!(request.is_suppressed("arize"), !additive);
}

#[test]
fn is_suppressed_ignores_unnamed_destinations() {
    let request = RequestOtelScope {
        destinations: vec![destination_with("http://collector", &[], None)],
        additive: false,
    };
    assert!(!request.is_suppressed("arize"));
    assert!(!request.is_suppressed(""));
}

#[test]
fn a_request_without_destinations_suppresses_nothing() {
    let scope = NoRequestScope.scope();
    assert!(scope.destinations.is_empty());
    assert!(!scope.additive);
    assert!(!scope.is_suppressed("any_backend"));
}
