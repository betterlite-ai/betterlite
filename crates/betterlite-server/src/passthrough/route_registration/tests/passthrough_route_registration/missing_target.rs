use super::super::support::{parse_endpoint, register_with, registry_keys, route_count, shared};

#[test]
fn missing_target_keeps_later_endpoints() {
    let routes = shared();
    let no_target = parse_endpoint(r#"{"id":"e1","path":"/a","methods":["GET"]}"#);
    let good =
        parse_endpoint(r#"{"id":"e2","path":"/b","target":"https://upstream","methods":["GET"]}"#);
    register_with(&routes, &[no_target, good], None, &[]);
    assert_eq!(registry_keys(&routes), ["e2:exact:/b:GET".to_owned()]);
}

#[test]
fn missing_target_keeps_its_orphan_path_across_reloads() {
    let routes = shared();
    let no_target = parse_endpoint(r#"{"id":"e1","path":"/custom","auth":true,"methods":["GET"]}"#);
    register_with(&routes, &[no_target], None, &[]);
    register_with(&routes, &[], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
}
