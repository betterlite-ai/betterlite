#![cfg(test)]
use betterlite_auth::pass_through_registry::{PassThroughMetadata, PassThroughRouteType};

use super::registered_route_keys;

fn metadata(
    endpoint_id: &str,
    route_type: PassThroughRouteType,
    path: &str,
) -> PassThroughMetadata {
    PassThroughMetadata {
        endpoint_id: endpoint_id.to_owned(),
        route_type,
        path: path.to_owned(),
        methods: Vec::new(),
        auth: None,
    }
}

#[test]
fn registered_route_keys_lists_every_entry_in_order() {
    let registrations = vec![
        (
            "a:exact:/foo:GET".to_owned(),
            metadata("a", PassThroughRouteType::Exact, "/foo"),
        ),
        (
            "b:exact:/bar:GET".to_owned(),
            metadata("b", PassThroughRouteType::Exact, "/bar"),
        ),
    ];
    assert_eq!(
        registered_route_keys(&registrations),
        vec!["a:exact:/foo:GET".to_owned(), "b:exact:/bar:GET".to_owned()]
    );
}
