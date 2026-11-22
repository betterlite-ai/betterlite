use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::wildcard_catalog::PatternCatalog;
use indexmap::IndexMap;

use super::super::ports::{ModelAccessGroupCache, ModelNameSource};
use super::fixtures::auth::recording_auth_invalidation;
use super::names;

fn catalog_deployment(model_name: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: JsonObject::new(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

#[test]
fn catalog_source_reports_live_names() {
    let catalog = DeploymentCatalog {
        models: vec![catalog_deployment("gpt-4o"), catalog_deployment("gpt-4")],
        model_names: BTreeSet::new(),
        deployment_names: Vec::new(),
        provider_default_ids: Vec::new(),
        provider_unresolved: Vec::new(),
        pattern_router: PatternCatalog::default(),
        team_pattern_routers: BTreeMap::new(),
        id_index: IndexMap::new(),
        name_index: BTreeMap::new(),
        team_index: BTreeMap::new(),
    };
    assert_eq!(
        catalog.model_names().expect("the served names"),
        Some(names(&["gpt-4o", "gpt-4"]))
    );
}

#[tokio::test]
async fn auth_cache_broadcasts_evictions() {
    let (recorder, invalidation) = recording_auth_invalidation();
    let keys = [
        "model_access_group:group-a".to_owned(),
        "model_access_group_registry".to_owned(),
    ];
    <AuthInvalidation as ModelAccessGroupCache>::evict_group_cache_keys(&invalidation, &keys).await;
    assert_eq!(recorder.broadcasts(), vec![keys.to_vec()]);
}
