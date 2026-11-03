#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn absent_public_routes() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings: {}\n")
    .expect("config should load");
    assert!(config.sections.auth_route_patterns.public_routes.is_empty());
}

#[test]
fn public_routes_list_loaded() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  public_routes: ['/api/*', '/health']\n")
    .expect("config should load");
    assert_eq!(
        config.sections.auth_route_patterns.public_routes,
        ["/api/*", "/health"]
    );
}
