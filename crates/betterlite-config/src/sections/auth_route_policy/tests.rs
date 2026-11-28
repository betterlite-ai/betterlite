#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_route_policy::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)?
    .sections
    .auth_route_policy)
}

#[test]
fn admin_routes_absent() {
    assert_eq!(load("{}").unwrap(), Settings::default());
    let null = load("general_settings:\n  admin_only_routes: null\n").unwrap();
    assert_eq!(null.admin_only_routes, None);
    let elsewhere = load("general_settings:\n  master_key: sk-test\n").unwrap();
    assert_eq!(elsewhere.admin_only_routes, None);
}

#[test]
fn admin_routes_empty() {
    let settings = load("general_settings:\n  admin_only_routes: []\n").unwrap();
    assert_eq!(settings.admin_only_routes, Some(Vec::new()));
}

#[test]
fn admin_routes_exact() {
    let settings = load(
        "general_settings:\n  admin_only_routes: ['/user/new', '/team/permissions_bulk_update']\n",
    )
    .unwrap();
    assert_eq!(
        settings.admin_only_routes,
        Some(vec![
            "/user/new".to_owned(),
            "/team/permissions_bulk_update".to_owned()
        ])
    );
}

#[test]
fn admin_routes_reject_a_non_list() {
    let error = load("general_settings:\n  admin_only_routes: /user/new\n").unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}
