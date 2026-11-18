use super::super::support::{parse_endpoint, register_with, registry_keys, shared};

#[test]
fn missing_target_keeps_later_endpoints() {
    let routes = shared();
    let no_target = parse_endpoint(r#"{"id":"e1","path":"/a","methods":["GET"]}"#);
    let good =
        parse_endpoint(r#"{"id":"e2","path":"/b","target":"https://upstream","methods":["GET"]}"#);
    register_with(&routes, &[no_target, good], None, &[]);
    assert_eq!(registry_keys(&routes), ["e2:exact:/b:GET".to_owned()]);
}
