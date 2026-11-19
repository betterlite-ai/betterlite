use super::super::support::{
    lock_state, method_set, parse_endpoint, register_with, route_count, shared, subpath_template,
};

#[test]
fn subpath_route_strips_a_trailing_slash_only_from_the_wildcard() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom/","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom/*"), 1);
    let state = lock_state(&routes);
    assert_eq!(
        state
            .table
            .colliding_routes(&subpath_template("/custom/"), &method_set(&["GET"]))
            .len(),
        1
    );
    drop(state);
}
