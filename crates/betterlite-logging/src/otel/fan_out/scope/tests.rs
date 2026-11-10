#![cfg(test)]

use rstest::rstest;

use super::RequestOtelScope;
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
