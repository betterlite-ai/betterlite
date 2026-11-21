use betterlite_db::resource_rows::access_group::AccessGroupTable;
use tracing::Level;

use super::support::{Levels, assert_http, json, lookup, without_database, world};
use crate::auth_team_lookup::access::get_access_object;
use crate::auth_team_lookup::error::{Detail, TeamFailure, TeamLookupError};

fn group_row(name: &str) -> String {
    format!(
        r#"{{"access_group_id": "g1", "access_group_name": "{name}", "access_model_names": ["gpt-4o"], "created_by": "a", "updated_by": "a", "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"}}"#
    )
}

fn group(name: &str) -> AccessGroupTable {
    serde_json::from_str(&group_row(name)).unwrap()
}

fn failure_cause(error: &TeamLookupError) -> &TeamFailure {
    match error {
        TeamLookupError::AccessGroupFailure {
            access_group_id,
            cause,
        }
        | TeamLookupError::AccessPolicyUnavailable {
            access_group_id,
            cause,
        } => {
            assert_eq!(access_group_id, "g1");
            cause.as_ref()
        }
        TeamLookupError::NoDatabase
        | TeamLookupError::CacheRead(_)
        | TeamLookupError::NotInCache { .. }
        | TeamLookupError::TeamNotFound { .. }
        | TeamLookupError::TeamUnavailable { .. }
        | TeamLookupError::AliasNotFound { .. }
        | TeamLookupError::AliasAmbiguous { .. }
        | TeamLookupError::AliasFailure { .. }
        | TeamLookupError::AccessGroupNotFound { .. } => {
            panic!("an access group failure is expected, got {error:?}")
        }
    }
}

#[tokio::test]
async fn access_cache_hit() {
    let world = world();
    world
        .lock()
        .groups
        .insert("access_group_id:g1".to_owned(), group("cached"));
    world.lock().group_rows.push(json(&group_row("stored")));
    let unwired = get_access_object(&without_database(&world), "g1", false)
        .await
        .unwrap_err();
    assert!(matches!(unwired, TeamLookupError::NoDatabase));
    assert!(
        world.events().is_empty(),
        "the cache is never read without a database"
    );
    let found = get_access_object(&lookup(&world, 60), "g1", false)
        .await
        .unwrap();
    assert_eq!(found.access_group_name, "cached");
    assert_eq!(world.events(), ["read access_group_id:g1"]);
}

#[tokio::test]
async fn access_writer_skips_cache() {
    let world = world();
    world
        .lock()
        .groups
        .insert("access_group_id:g1".to_owned(), group("cached"));
    world.lock().group_rows.push(json(&group_row("stored")));
    let found = get_access_object(&lookup(&world, 60), "g1", true)
        .await
        .unwrap();
    assert_eq!(found, group("stored"));
    assert_eq!(
        world.events(),
        ["access g1 writer=true", "write access_group_id:g1 ttl=600"]
    );
    world.lock().events.clear();
    world.lock().groups.clear();
    get_access_object(&lookup(&world, 60), "g1", false)
        .await
        .unwrap();
    let expected = [
        "read access_group_id:g1",
        "access g1 writer=false",
        "write access_group_id:g1 ttl=600",
    ];
    assert_eq!(world.events(), expected);
}

#[tokio::test]
async fn access_missing_404() {
    let world = world();
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_access_object(&lookup(&world, 60), "g1", false)
        .await
        .unwrap_err();
    drop(guard);
    assert!(
        matches!(&error, TeamLookupError::AccessGroupNotFound { access_group_id } if access_group_id == "g1")
    );
    assert_http(&error, 404, Detail::Error);
    assert!(levels.seen().is_empty());
    assert_eq!(
        world.events(),
        ["read access_group_id:g1", "access g1 writer=false"]
    );
}

#[tokio::test]
async fn access_read_failure_404() {
    let world = world();
    world.lock().failing.insert("access".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_access_object(&lookup(&world, 60), "g1", false)
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(error, TeamLookupError::AccessGroupFailure { .. }));
    assert!(matches!(failure_cause(&error), TeamFailure::Store(_)));
    assert_http(&error, 404, Detail::Error);
    assert_eq!(levels.seen(), vec![Level::ERROR]);
}

#[tokio::test]
async fn access_writer_failure_503() {
    let world = world();
    world.lock().failing.insert("access".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let read = get_access_object(&lookup(&world, 60), "g1", true)
        .await
        .unwrap_err();
    assert!(matches!(
        read,
        TeamLookupError::AccessPolicyUnavailable { .. }
    ));
    assert!(matches!(failure_cause(&read), TeamFailure::Store(_)));
    assert_http(&read, 503, Detail::Text);
    assert_eq!(read.to_string(), "Access group policy is unavailable");

    world.lock().failing.clear();
    world
        .lock()
        .group_rows
        .push(json(r#"{"access_group_id": "g1"}"#));
    let invalid = get_access_object(&lookup(&world, 60), "g1", true)
        .await
        .unwrap_err();
    assert!(matches!(failure_cause(&invalid), TeamFailure::AccessRow(_)));
    assert_http(&invalid, 503, Detail::Text);

    world.lock().group_rows = vec![json(&group_row("stored"))];
    world
        .lock()
        .failing
        .insert("write access_group_id:g1".to_owned());
    let cache = get_access_object(&lookup(&world, 60), "g1", true)
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(failure_cause(&cache), TeamFailure::Cache(_)));
    assert_eq!(
        levels.seen(),
        vec![Level::ERROR, Level::ERROR, Level::ERROR]
    );
}

#[tokio::test]
async fn access_http_error_preserved() {
    let world = world();
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_access_object(&lookup(&world, 60), "g1", true)
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(error, TeamLookupError::AccessGroupNotFound { .. }));
    assert_http(&error, 404, Detail::Error);
    assert!(
        levels.seen().is_empty(),
        "the 404 passes the writer's 503 branch unlogged"
    );
    assert_eq!(world.events(), ["access g1 writer=true"]);
}
