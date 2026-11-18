#![cfg(test)]

use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::deployments::router_models;
use crate::roi_settings::support::fake_catalog;

#[test]
fn router_models_sorts_and_deduplicates_the_router_names() {
    let mut catalog = fake_catalog();
    catalog.names = vec![
        "z".to_owned(),
        "a".to_owned(),
        "a".to_owned(),
        "m".to_owned(),
    ];
    let router: &dyn RouterCatalog = &catalog;
    assert_eq!(router_models(Some(router)), ["a", "m", "z"]);
}

#[test]
fn router_models_without_a_router_is_empty() {
    assert!(router_models(None).is_empty());
}
