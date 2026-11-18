#![cfg(test)]

use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::deployments::router_estimator_models;
use crate::roi_settings::support::deployment;
use crate::roi_settings::support::fake_catalog;

#[test]
fn router_estimator_models_reads_the_requested_model_group() {
    let mut catalog = fake_catalog();
    catalog.deployments = vec![deployment(Some("gpt-4o"), None), deployment(None, None)];
    let router: &dyn RouterCatalog = &catalog;
    let models = router_estimator_models(Some(router), "estimator");
    assert_eq!(
        models,
        vec![("gpt-4o".to_owned(), Some("openai".to_owned()))]
    );
    assert_eq!(
        catalog.requests.lock().expect("catalog lock").clone(),
        vec!["estimator".to_owned()]
    );
}

#[test]
fn router_estimator_models_without_a_router_is_empty() {
    assert!(router_estimator_models(None, "estimator").is_empty());
}
