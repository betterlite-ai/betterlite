#![cfg(test)]

use rstest::rstest;

use super::{PassThroughMetadata, PassThroughRegistry, PassThroughRouteType};

fn metadata(id: &str, methods: &[&str], auth: Option<bool>) -> PassThroughMetadata {
    PassThroughMetadata {
        endpoint_id: id.to_owned(),
        route_type: PassThroughRouteType::Exact,
        path: "/registered".to_owned(),
        methods: methods.iter().map(|method| (*method).to_owned()).collect(),
        auth,
    }
}

fn registry(root: &str, entries: Vec<(&str, PassThroughMetadata)>) -> PassThroughRegistry {
    PassThroughRegistry {
        root_path: root.to_owned(),
        registrations: entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    }
}

#[rstest]
#[case::empty_root("", "/root/path", "/root/path")]
#[case::slash_root("/", "/root/path", "/root/path")]
#[case::prefixed_path("/proxy", "/proxy/root/path", "/root/path")]
#[case::already_stripped_lookup("/proxy", "/root/path", "/root/path")]
#[case::exact_root_not_under_root("/proxy", "/proxy", "/proxy")]
#[case::similar_prefix_not_stripped("/proxy", "/proxy-other/path", "/proxy-other/path")]
#[case::trailing_root_slash_literal("/proxy/", "/proxy/root/path", "/proxy/root/path")]
fn normalizes_routes(#[case] root: &str, #[case] route: &str, #[case] expected: &str) {
    assert_eq!(registry(root, Vec::new()).lookup_route(route), expected);
}

#[test]
fn mapped_prefix_is_a_predicate_only() {
    let routes = registry("/proxy", Vec::new());
    assert!(routes.is_registered("/proxy/openai/chat/completions"));
    assert!(
        routes
            .get_registered("/proxy/openai/chat/completions", None)
            .is_none()
    );
}

#[test]
fn registration_matching_preserves_order_and_path_boundaries() {
    let routes = registry(
        "",
        vec![
            (
                "first:subpath:/registered",
                metadata("first", &[], Some(false)),
            ),
            (
                "second:exact:/registered",
                metadata("second", &[], Some(true)),
            ),
        ],
    );
    assert_eq!(
        routes
            .get_registered("/registered/child", None)
            .map(|item| item.endpoint_id.as_str()),
        Some("first")
    );
    assert!(!routes.is_registered("/registered-sibling"));
}

#[test]
fn methods_prefer_metadata_and_continue_to_later_matches() {
    let routes = registry(
        "",
        vec![
            (
                "first:exact:/registered:POST",
                metadata("first", &["GET"], Some(false)),
            ),
            (
                "second:exact:/registered:POST",
                metadata("second", &["POST"], Some(true)),
            ),
        ],
    );
    assert_eq!(
        routes
            .get_registered("/registered", Some("POST"))
            .map(|item| item.endpoint_id.as_str()),
        Some("second")
    );
    assert_eq!(routes.get_registered("/registered", Some("post")), None);
}

#[test]
fn old_keys_are_unrestricted_and_empty_key_methods_are_not_get() {
    let old = registry(
        "",
        vec![("old:exact:/registered", metadata("old", &[], None))],
    );
    assert!(old.get_registered("/registered", Some("GET")).is_some());
    let empty = registry(
        "",
        vec![("empty:exact:/registered:", metadata("empty", &[], None))],
    );
    assert!(empty.get_registered("/registered", Some("GET")).is_none());
}

#[test]
fn malformed_and_unknown_keys_do_not_match() {
    let routes = registry(
        "",
        vec![
            ("malformed:exact", metadata("malformed", &[], None)),
            ("unknown:other:/registered", metadata("unknown", &[], None)),
        ],
    );
    assert!(!routes.is_registered("/registered"));
    assert!(routes.get_registered("/registered", None).is_none());
}
