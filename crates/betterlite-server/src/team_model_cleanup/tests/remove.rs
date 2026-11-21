use betterlite_db::budget_permission_rows::JsonData;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;

use crate::team_model_cleanup::remove::{DeletedModel, remove_unbacked_team_models};

use super::support::{
    FakeCache, FakeStore, alias_row, aliases, context, deployment, names, serving,
};

fn deleted(model_name: &str, team_id: Option<&str>, public: Option<&str>) -> DeletedModel {
    DeletedModel {
        model_name: model_name.to_owned(),
        team_id: team_id.map(str::to_owned),
        team_public_model_name: public.map(str::to_owned),
    }
}

#[tokio::test]
async fn last_sibling_removed() {
    let store = FakeStore::new("t1", Some(names(&["pub", "other"])), Vec::new(), Vec::new());
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap()
    .expect("the team row is returned for cache invalidation");
    assert_eq!(
        updated.fields.get("team_id"),
        Some(&JsonData::String("t1".to_owned()))
    );
    assert_eq!(store.current_models(), Some(names(&["other"])));
    assert_eq!(cache.stored(), vec![("t1".to_owned(), names(&["other"]))]);
}

#[tokio::test]
async fn configured_sibling_kept() {
    let alias_rows = vec![alias_row(1, Some("t1"), Some(r#"{"legacy":"shared"}"#))];
    let store = FakeStore::new(
        "t1",
        Some(names(&["shared", "legacy"])),
        alias_rows,
        Vec::new(),
    );
    let cache = FakeCache::default();
    let router = serving(&["shared"]);
    let updated = remove_unbacked_team_models(
        &deleted("shared", Some("t1"), Some("shared")),
        &context(&store, &cache, Some(&router)),
    )
    .await
    .unwrap();
    assert!(updated.is_none());
    assert_eq!(store.current_models(), Some(names(&["shared", "legacy"])));
    assert!(store.saved_aliases.lock().unwrap().is_empty());
    assert!(cache.stored().is_empty());
}

#[tokio::test]
async fn stored_sibling_kept() {
    let deployments = vec![deployment(
        "model_name_t1_sibling",
        r#"{"team_id":"t1","team_public_model_name":"pub"}"#,
    )];
    let store = FakeStore::new("t1", Some(names(&["pub"])), Vec::new(), deployments);
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_none());
    assert_eq!(store.current_models(), Some(names(&["pub"])));
    assert!(cache.stored().is_empty());
}

#[tokio::test]
async fn a_deleted_model_without_a_team_is_ignored() {
    let store = FakeStore::new("t1", Some(names(&["pub"])), Vec::new(), Vec::new());
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", None, Some("pub")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_none());
    assert!(store.saved_models.lock().unwrap().is_empty());
    assert!(cache.stored().is_empty());
}

#[tokio::test]
async fn a_legacy_alias_name_is_removed_with_the_deleted_deployment() {
    let alias_rows = vec![alias_row(1, Some("t1"), Some(r#"{"legacy":"pub"}"#))];
    let store = FakeStore::new(
        "t1",
        Some(names(&["legacy", "other"])),
        alias_rows,
        Vec::new(),
    );
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), None),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_some());
    assert_eq!(store.current_models(), Some(names(&["other"])));
    assert_eq!(store.remaining_aliases(), vec![Some(aliases("{}"))]);
}

#[tokio::test]
async fn an_alias_of_another_team_is_not_removed_from_the_models() {
    let alias_rows = vec![alias_row(1, Some("t2"), Some(r#"{"foreign":"pub"}"#))];
    let store = FakeStore::new(
        "t1",
        Some(names(&["foreign", "pub"])),
        alias_rows,
        Vec::new(),
    );
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_some());
    assert_eq!(store.current_models(), Some(names(&["foreign"])));
}

#[tokio::test]
async fn an_empty_public_name_is_removed_as_given() {
    let store = FakeStore::new("t1", Some(names(&["", "other"])), Vec::new(), Vec::new());
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_some());
    assert_eq!(store.current_models(), Some(names(&["other"])));
}

#[tokio::test]
async fn no_candidate_leaves_the_team_row_alone() {
    let store = FakeStore::new("t1", Some(names(&["pub"])), Vec::new(), Vec::new());
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), None),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_none());
    assert!(store.saved_models.lock().unwrap().is_empty());
    assert!(cache.stored().is_empty());
}

#[tokio::test]
async fn a_missing_team_row_keeps_the_alias_cleanup() {
    let alias_rows = vec![alias_row(1, Some("t1"), Some(r#"{"role":"pub"}"#))];
    let store = FakeStore::new("t1", None, alias_rows, Vec::new());
    let cache = FakeCache::default();
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, None),
    )
    .await
    .unwrap();
    assert!(updated.is_none());
    assert_eq!(store.remaining_aliases(), vec![Some(aliases("{}"))]);
    assert!(store.saved_models.lock().unwrap().is_empty());
    assert!(cache.stored().is_empty());
}

#[tokio::test]
async fn a_deployment_catalog_serves_only_the_names_it_indexes() {
    let mut catalog = DeploymentCatalog::default();
    catalog.name_index.insert("shared".to_owned(), Vec::new());
    let store = FakeStore::new(
        "t1",
        Some(names(&["shared", "pub"])),
        Vec::new(),
        Vec::new(),
    );
    let cache = FakeCache::default();
    let kept = remove_unbacked_team_models(
        &deleted("shared", Some("t1"), Some("shared")),
        &context(&store, &cache, Some(&catalog)),
    )
    .await
    .unwrap();
    assert!(kept.is_none());
    assert_eq!(store.current_models(), Some(names(&["shared", "pub"])));
    let removed = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, Some(&catalog)),
    )
    .await
    .unwrap();
    assert!(removed.is_some());
    assert_eq!(store.current_models(), Some(names(&["shared"])));
}

#[tokio::test]
async fn a_router_serving_a_candidate_keeps_it() {
    let alias_rows = vec![alias_row(1, Some("t1"), Some(r#"{"role":"pub"}"#))];
    let store = FakeStore::new("t1", Some(names(&["role", "pub"])), alias_rows, Vec::new());
    let cache = FakeCache::default();
    let router = serving(&["role"]);
    let updated = remove_unbacked_team_models(
        &deleted("pub", Some("t1"), Some("pub")),
        &context(&store, &cache, Some(&router)),
    )
    .await
    .unwrap();
    assert!(updated.is_some());
    assert_eq!(store.current_models(), Some(names(&["role"])));
}
