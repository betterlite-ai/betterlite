#![cfg(test)]

use super::{
    NamedEndpoint, Route, RouteInfo, RouteInspectionError, get_app_routes,
    get_routes_for_mounted_app,
};

struct Endpoint(Option<&'static str>);

impl NamedEndpoint for Endpoint {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.0)
    }
}

#[test]
fn regular_route_keeps_nulls_and_endpoint_name() {
    let route = Route {
        path: Some("/chat".to_owned()),
        methods: Some(vec!["POST".to_owned()]),
        name: Some("chat".to_owned()),
        endpoint: Some("present".to_owned()),
        app: None,
    };
    let info = get_app_routes(&route, &Endpoint(Some("handler")));
    assert_eq!(info[0].endpoint.as_deref(), Some("handler"));
    assert_eq!(info[0].mounted_app, None);
}

#[test]
fn regular_route_without_endpoint_keeps_null_fields() {
    let info = get_app_routes(&Route::default(), &Endpoint(Some("handler")));
    assert_eq!(
        info,
        vec![RouteInfo {
            path: None,
            methods: None,
            name: None,
            endpoint: None,
            mounted_app: None,
        }]
    );
}

#[test]
fn mounted_routes_join_prefix_and_default_methods() {
    let route = Route {
        path: Some("/v1/".to_owned()),
        app: Some(vec![Route {
            path: Some("/models".to_owned()),
            endpoint: Some("list_models".to_owned()),
            methods: None,
            name: None,
            app: None,
        }]),
        methods: None,
        name: None,
        endpoint: None,
    };
    let info = get_routes_for_mounted_app(&route);
    assert_eq!(info[0].path.as_deref(), Some("/v1/models"));
    assert_eq!(
        info[0].methods,
        Some(vec!["GET".to_owned(), "POST".to_owned()])
    );
    assert_eq!(info[0].mounted_app, Some(true));
}

#[test]
fn mounted_app_with_no_routes_is_empty() {
    assert!(get_routes_for_mounted_app(&Route::default()).is_empty());
}

#[test]
fn mounted_routes_skip_routes_without_endpoint_or_nested_app() {
    let route = Route {
        path: None,
        methods: None,
        name: None,
        endpoint: None,
        app: Some(vec![Route {
            path: None,
            methods: None,
            name: None,
            endpoint: None,
            app: None,
        }]),
    };
    assert!(get_routes_for_mounted_app(&route).is_empty());
}
