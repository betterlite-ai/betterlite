use rstest::rstest;

use crate::login_return::origin::{ReturnToError, validate_return_to};
use crate::login_return::return_destination::is_same_origin_return_path;

#[derive(Clone, Copy, Debug)]
enum Expected {
    Accepted,
    Ignored,
    Rejected,
    ParseError,
}

#[rstest]
#[case::relative_single_slash("/v1/models?tab=keys", true)]
#[case::relative_double_slash("//evil.example/v1", false)]
#[case::relative_backslash("/\\evil.example/v1", false)]
#[case::relative_controls("/a\tb", false)]
#[case::relative_percent_literal("/%2F..%2Fadmin", true)]
#[case::relative_unicode("/café/keys", true)]
fn same_origin_return_paths(#[case] return_to: &str, #[case] expected: bool) {
    assert_eq!(
        is_same_origin_return_path(return_to),
        expected,
        "{return_to:?}"
    );
}

#[rstest]
#[case::space("/a b")]
#[case::del("/a\u{7f}b")]
#[case::null_byte("/a\u{0}b")]
#[case::unit_separator("/a\u{1f}b")]
#[case::missing_leading_slash("ui/keys")]
#[case::empty("")]
fn relative_path_rejections(#[case] return_to: &str) {
    assert!(!is_same_origin_return_path(return_to), "{return_to:?}");
}

#[rstest]
#[case::missing_control_plane("https://cp.example/ui", None, Expected::Ignored)]
#[case::same_origin_casefold(
    "https://CP.Example/ui",
    Some("https://cp.example"),
    Expected::Accepted
)]
#[case::default_ports(
    "https://cp.example:443/ui",
    Some("https://cp.example"),
    Expected::Accepted
)]
#[case::non_https_port_80("ws://cp.example:80/ui", Some("ws://cp.example"), Expected::Accepted)]
#[case::different_origin(
    "https://cp.example:8443/ui",
    Some("https://cp.example"),
    Expected::Rejected
)]
#[case::malformed_port(
    "https://cp.example:abc/ui",
    Some("https://cp.example"),
    Expected::ParseError
)]
fn control_plane_origins(
    #[case] return_to: &str,
    #[case] control_plane_url: Option<&str>,
    #[case] expected: Expected,
) {
    let result = validate_return_to(return_to, control_plane_url);
    match expected {
        Expected::Accepted => assert!(result.unwrap(), "{return_to:?}"),
        Expected::Ignored => assert!(!result.unwrap(), "{return_to:?}"),
        Expected::Rejected => {
            let error = result.expect_err("the origin does not match");
            assert_eq!(error.status(), Some(400));
            assert!(matches!(error, ReturnToError::MismatchedOrigin));
        }
        Expected::ParseError => {
            let error = result.expect_err("the malformed port fails the parse");
            assert_eq!(error.status(), None);
            assert!(matches!(error, ReturnToError::Parse(_)));
        }
    }
}

#[rstest]
#[case::userinfo_is_not_part_of_the_origin(
    "https://user:pw@cp.example/ui",
    "https://cp.example",
    Expected::Accepted
)]
#[case::wss_defaults_to_port_80("wss://cp.example/ui", "wss://cp.example:443", Expected::Rejected)]
fn control_plane_origin_details(
    #[case] return_to: &str,
    #[case] control_plane_url: &str,
    #[case] expected: Expected,
) {
    let result = validate_return_to(return_to, Some(control_plane_url));
    match expected {
        Expected::Accepted => assert!(result.unwrap(), "{return_to:?}"),
        Expected::Rejected => {
            let error = result.expect_err("the origin does not match");
            assert_eq!(error.status(), Some(400));
            assert!(matches!(error, ReturnToError::MismatchedOrigin));
        }
        Expected::Ignored | Expected::ParseError => {
            panic!("this table names only accepted or rejected origins")
        }
    }
}

#[rstest]
#[case::another_scheme("http://cp.example/ui", "https://cp.example")]
#[case::another_port("https://cp.example:80/ui", "https://cp.example")]
#[case::another_host("https://other.example/ui", "https://cp.example")]
#[case::trailing_dot("https://cp.example./ui", "https://cp.example")]
fn matching_hostname_is_not_enough(#[case] return_to: &str, #[case] control_plane_url: &str) {
    let error = validate_return_to(return_to, Some(control_plane_url))
        .expect_err("the full origin must match");
    assert_eq!(error.status(), Some(400));
    assert!(matches!(error, ReturnToError::MismatchedOrigin));
}

#[test]
fn malformed_control_plane_port_propagates() {
    let error = validate_return_to("https://cp.example/ui", Some("https://cp.example:65536"))
        .expect_err("an out-of-range control plane port fails the parse");
    assert_eq!(error.status(), None);
    assert!(matches!(error, ReturnToError::Parse(_)));
}

#[test]
fn default_port_depends_on_the_scheme() {
    assert!(
        validate_return_to("http://cp.example/ui", Some("http://cp.example:80"))
            .expect("the http origin matches")
    );
    assert!(
        validate_return_to("https://cp.example/ui", Some("https://cp.example:443"))
            .expect("the https origin matches")
    );
    assert_eq!(
        validate_return_to("http://cp.example:443/ui", Some("http://cp.example"))
            .expect_err("443 is not the default port for http")
            .status(),
        Some(400)
    );
}
