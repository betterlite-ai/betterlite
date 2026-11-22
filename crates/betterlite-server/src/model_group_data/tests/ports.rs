use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::alias::AliasTarget;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use super::super::ports::{LiveModelNames, ModelAccessGroupCache, ModelNameSource as _};
use super::fixtures::auth::recording_auth_invalidation;
use super::fixtures::router::{alias_map, catalog_handle, replace_catalog, routing_groups};
use super::names;

#[test]
fn live_source_reports_catalog_alias_and_group_names() {
    let catalog = catalog_handle(&["gpt-4o"]);
    let aliases = alias_map(&[
        ("fast", AliasTarget::Name("gpt-4o".to_owned())),
        (
            "hidden",
            AliasTarget::Item {
                model: Some(Json::Text("gpt-4o".to_owned())),
                hidden: Some(Json::Bool(true)),
            },
        ),
    ]);
    let groups = routing_groups("pool", &["gpt-4o"]);
    let source = LiveModelNames {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
    };
    assert_eq!(
        source.model_names().expect("the served names"),
        Some(names(&["fast", "pool", "gpt-4o"]))
    );
}

#[test]
fn live_source_reads_the_swapped_catalog() {
    let catalog = catalog_handle(&["alpha"]);
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let source = LiveModelNames {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
    };
    assert_eq!(
        source.model_names().expect("the first names"),
        Some(names(&["alpha"]))
    );
    replace_catalog(&catalog, &["beta"]);
    assert_eq!(
        source.model_names().expect("the rebound names"),
        Some(names(&["beta"]))
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
