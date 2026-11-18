use super::super::support::{
    lock_state, parse_endpoint, register_with, registry_keys, route_count, shared,
};

#[test]
fn stale_removal_drops_a_route_from_a_prior_load() {
    let routes = shared();
    let first =
        parse_endpoint(r#"{"id":"e1","path":"/a","target":"https://upstream","methods":["GET"]}"#);
    let second =
        parse_endpoint(r#"{"id":"e2","path":"/b","target":"https://upstream","methods":["GET"]}"#);
    register_with(&routes, &[first.clone(), second], None, &[]);
    register_with(&routes, &[first], None, &[]);
    assert_eq!(registry_keys(&routes), ["e1:exact:/a:GET".to_owned()]);
    let state = lock_state(&routes);
    assert_eq!(state.endpoints.len(), 1);
    assert_eq!(state.table.order().len(), 2);
}

#[test]
fn stale_removal_keeps_a_path_shared_with_a_live_endpoint() {
    let routes = shared();
    let first = parse_endpoint(
        r#"{"id":"e1","path":"/shared","target":"https://upstream","auth":true,"methods":["GET"]}"#,
    );
    let second = parse_endpoint(
        r#"{"id":"e2","path":"/shared","target":"https://upstream","auth":true,"methods":["POST"]}"#,
    );
    register_with(&routes, &[first, second.clone()], None, &[]);
    register_with(&routes, &[second], None, &[]);
    assert_eq!(registry_keys(&routes), ["e2:exact:/shared:POST".to_owned()]);
    assert_eq!(route_count(&routes, "/shared"), 1);
}

#[test]
fn stale_removal_keeps_a_base_openai_route() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/v1/chat/completions","target":"https://upstream","auth":true,"methods":["POST"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    register_with(&routes, &[], None, &[]);
    assert!(registry_keys(&routes).is_empty());
    assert_eq!(route_count(&routes, "/v1/chat/completions"), 1);
}

#[test]
fn stale_removal_removes_an_unowned_path() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    register_with(&routes, &[], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
}

#[test]
fn stale_removal_removes_an_unowned_wildcard() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    register_with(&routes, &[], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
    assert_eq!(route_count(&routes, "/custom/*"), 0);
}

#[test]
fn stale_removal_keeps_a_wildcard_owned_by_a_live_subpath() {
    let routes = shared();
    let first = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    let second = parse_endpoint(
        r#"{"id":"e2","path":"/custom","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[first, second.clone()], None, &[]);
    register_with(&routes, &[second], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        [
            "e2:exact:/custom:GET".to_owned(),
            "e2:subpath:/custom:GET".to_owned()
        ]
    );
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert_eq!(route_count(&routes, "/custom/*"), 1);
}

#[test]
fn stale_removal_removes_a_wildcard_when_only_an_exact_route_is_live() {
    let routes = shared();
    let first = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    let second = parse_endpoint(
        r#"{"id":"e2","path":"/custom","target":"https://upstream","auth":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[first, second.clone()], None, &[]);
    register_with(&routes, &[second], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert_eq!(route_count(&routes, "/custom/*"), 0);
}
