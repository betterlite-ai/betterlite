#![cfg(test)]

use super::parse_route;
use crate::routes::{RouteMetadataError, get_call_types_for_route};

#[rstest::rstest]
#[case::bytes_at_limit("x".repeat(4096), true)]
#[case::bytes_over_limit("x".repeat(4097), false)]
#[case::segments_at_limit(vec!["x"; 256].join("/"), true)]
#[case::segments_over_limit(vec!["x"; 257].join("/"), false)]
#[case::empty(String::new(), true)]
fn call_route_metadata_route_metadata_tests_behavior(#[case] route: String, #[case] valid: bool) {
    assert_eq!(parse_route(&route).is_ok(), valid);
    assert_eq!(get_call_types_for_route(&route).is_ok(), valid);
    if route.len() > 4096 {
        assert!(matches!(
            parse_route(&route),
            Err(RouteMetadataError::RouteTooLong)
        ));
    } else if !valid {
        assert!(matches!(
            parse_route(&route),
            Err(RouteMetadataError::TooManySegments)
        ));
    } else {
        assert!(valid);
    }
}
