#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use betterlite_auth::auth_route_patterns::public_routes::is_public_route;
use rstest::rstest;

#[test]
fn absent_public_routes() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings: {}\n")
    .expect("config should load");
    assert!(config.sections.auth_route_patterns.public_routes.is_empty());
    assert!(!is_public_route(
        "/api/users",
        &config.sections.auth_route_patterns.public_routes
    ));
}

#[test]
fn public_routes_list_loaded() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  public_routes: ['/api/*', '/health']\n")
    .expect("config should load");
    assert!(is_public_route(
        "/health",
        &config.sections.auth_route_patterns.public_routes
    ));
    assert!(is_public_route(
        "/api/users",
        &config.sections.auth_route_patterns.public_routes
    ));
    assert!(!is_public_route(
        "/apiary/users",
        &config.sections.auth_route_patterns.public_routes
    ));
    assert_eq!(
        config.sections.auth_route_patterns.public_routes,
        ["/api/*", "/health"]
    );
}

#[rstest]
#[case::scalar("public_routes: /api/*")]
#[case::nonstring_entry("public_routes: [17]")]
fn invalid_public_routes(#[case] setting: &str) {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!("general_settings:\n  {setting}\n"))
    .unwrap_err();
    assert!(error.to_string().contains("public_routes"));
}

#[rstest]
#[case::null_routes("general_settings: {public_routes: null}")]
#[case::null_general("general_settings: null")]
#[case::empty_list("general_settings: {public_routes: []}")]
fn public_routes_default(#[case] yaml: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    assert!(config.sections.auth_route_patterns.public_routes.is_empty());
}

#[test]
fn top_level_public_routes_is_unsupported() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("public_routes: ['/api/*']")
    .unwrap_err();
    assert!(error.to_string().contains("public_routes"));
}
