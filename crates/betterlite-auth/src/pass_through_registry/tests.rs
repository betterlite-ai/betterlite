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
#[case::trailing_root_double_slash("/proxy/", "/proxy//root/path", "/root/path")]
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

#[rstest]
#[case::mapped_prefix_without_boundary("", "/openai-other", true)]
#[case::mapped_slash_root("/", "/openai-other", true)]
#[case::mapped_requires_normalization("/proxy", "/openai-other", false)]
#[case::mapped_prefixed("/proxy", "/proxy/openai-other", true)]
fn mapped_routes(#[case] root: &str, #[case] route: &str, #[case] expected: bool) {
    let routes = registry(root, Vec::new());
    assert_eq!(routes.is_registered(route), expected);
    assert_eq!(routes.get_registered(route, None), None);
}

#[rstest]
#[case::exact_match("first:exact:/registered", "/registered", true)]
#[case::exact_child_denied("first:exact:/registered", "/registered/child", false)]
#[case::subpath_self("first:subpath:/registered", "/registered", true)]
#[case::subpath_child("first:subpath:/registered", "/registered/child", true)]
#[case::subpath_sibling_denied("first:subpath:/registered", "/registered-other", false)]
fn registration_paths(#[case] key: &str, #[case] route: &str, #[case] expected: bool) {
    let record = metadata("first", &[], None);
    let routes = registry("/proxy", vec![(key, record.clone())]);
    assert_eq!(routes.is_registered(route), expected);
    assert_eq!(
        routes.get_registered(route, None),
        expected.then_some(&record)
    );
}

#[rstest]
#[case::metadata_methods_win("first:exact:/registered:POST", &["GET"], Some("GET"), true)]
#[case::metadata_denies_key_method("first:exact:/registered:POST", &["GET"], Some("POST"), false)]
#[case::key_methods_fallback("first:exact:/registered:GET,POST", &[], Some("POST"), true)]
#[case::split_at_most_three("first:exact:/registered:GET:POST", &[], Some("GET"), false)]
#[case::absent_method_bypass("first:exact:/registered:POST", &["GET"], None, true)]
#[case::empty_key_method_matches("first:exact:/registered:", &[], Some(""), true)]
fn registration_methods(
    #[case] key: &str,
    #[case] methods: &[&str],
    #[case] method: Option<&str>,
    #[case] expected: bool,
) {
    let record = metadata("first", methods, None);
    let routes = registry("", vec![(key, record.clone())]);
    assert!(routes.is_registered("/registered"));
    assert_eq!(
        routes.get_registered("/registered", method),
        expected.then_some(&record)
    );
}

#[rstest]
#[case::absent(None)]
#[case::disabled(Some(false))]
#[case::enabled(Some(true))]
fn auth_metadata_preserved(#[case] auth: Option<bool>) {
    let first = metadata("first", &[], auth);
    let routes = registry(
        "",
        vec![
            ("first:exact:/registered", first.clone()),
            ("second:exact:/registered", metadata("second", &[], None)),
        ],
    );
    assert_eq!(routes.get_registered("/registered", None), Some(&first));
}
