use betterlite_auth::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};

use crate::passthrough_route_registry::listing::registered_route_keys;

use super::super::remove_endpoint_routes;

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
fn removes_registry_entries_for_the_given_endpoint_only() {
    let mut registrations = vec![
        (
            "a:exact:/foo:GET".to_owned(),
            metadata("a", PassThroughRouteType::Exact, "/foo"),
        ),
        (
            "b:exact:/bar:GET".to_owned(),
            metadata("b", PassThroughRouteType::Exact, "/bar"),
        ),
    ];
    let mut openai_routes = vec!["/foo".to_owned(), "/bar".to_owned()];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "a");
    assert_eq!(
        registered_route_keys(&registrations),
        vec!["b:exact:/bar:GET"]
    );
}

#[test]
fn removes_exact_path_from_openai_routes() {
    let mut registrations = vec![(
        "a:exact:/custom:GET".to_owned(),
        metadata("a", PassThroughRouteType::Exact, "/custom"),
    )];
    let mut openai_routes = vec!["/custom".to_owned(), "/other".to_owned()];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "a");
    assert_eq!(openai_routes, vec!["/other".to_owned()]);
}

#[test]
fn removes_both_path_and_wildcard_for_subpath_type() {
    let mut registrations = vec![(
        "a:subpath:/custom:GET".to_owned(),
        metadata("a", PassThroughRouteType::Subpath, "/custom"),
    )];
    let mut openai_routes = vec![
        "/custom".to_owned(),
        "/custom/*".to_owned(),
        "/other".to_owned(),
    ];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "a");
    assert_eq!(openai_routes, vec!["/other".to_owned()]);
}

#[test]
fn removes_only_one_occurrence_of_a_duplicated_path() {
    let mut registrations = vec![(
        "a:exact:/dup:GET".to_owned(),
        metadata("a", PassThroughRouteType::Exact, "/dup"),
    )];
    let mut openai_routes = vec!["/dup".to_owned(), "/dup".to_owned()];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "a");
    assert_eq!(openai_routes, vec!["/dup".to_owned()]);
}

#[test]
fn unknown_endpoint_id_is_a_noop() {
    let mut registrations = vec![(
        "a:exact:/foo:GET".to_owned(),
        metadata("a", PassThroughRouteType::Exact, "/foo"),
    )];
    let mut openai_routes = vec!["/foo".to_owned()];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "missing");
    assert_eq!(
        registered_route_keys(&registrations),
        vec!["a:exact:/foo:GET"]
    );
    assert_eq!(openai_routes, vec!["/foo".to_owned()]);
}

#[test]
fn remaining_routes_stay_registered_in_b2302s_registry() {
    let mut registrations = vec![
        (
            "a:exact:/foo:GET".to_owned(),
            metadata("a", PassThroughRouteType::Exact, "/foo"),
        ),
        (
            "b:exact:/bar:GET".to_owned(),
            metadata("b", PassThroughRouteType::Exact, "/bar"),
        ),
    ];
    let mut openai_routes = vec!["/foo".to_owned(), "/bar".to_owned()];
    remove_endpoint_routes(&mut registrations, &mut openai_routes, "a");
    let registry = PassThroughRegistry {
        root_path: String::new(),
        registrations,
    };
    assert!(!registry.is_registered("/foo"));
    assert!(registry.is_registered("/bar"));
}
