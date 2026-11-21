use tracing::Level;

use super::support::{
    Levels, assert_http, cached_team, json, lookup, team_row, without_database, world,
};
use crate::auth_team_lookup::alias::get_team_object_by_alias;
use crate::auth_team_lookup::error::{Detail, TeamFailure, TeamLookupError};

fn assert_failure(error: &TeamLookupError) -> &TeamFailure {
    assert_http(error, 500, Detail::Error);
    let TeamLookupError::AliasFailure { team_alias, cause } = error else {
        panic!("the alias 500 is expected, got {error:?}");
    };
    assert_eq!(team_alias, "a1");
    cause.as_ref()
}

#[tokio::test]
async fn alias_cache_hit() {
    let world = world();
    world
        .lock()
        .teams
        .insert("team_alias:a1".to_owned(), cached_team("t9"));
    world.lock().team_rows.push(team_row("t1", "a1", None));
    let unwired = get_team_object_by_alias(&without_database(&world), "a1")
        .await
        .unwrap_err();
    assert!(matches!(unwired, TeamLookupError::NoDatabase));
    assert!(
        world.events().is_empty(),
        "the alias cache is never read without a database"
    );
    let team = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap();
    assert_eq!(team.team.fields.base.team_id, "t9");
    assert_eq!(world.events(), ["read team_alias:a1"]);
}

#[tokio::test]
async fn alias_missing_404() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "other", None));
    let error = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap_err();
    assert!(matches!(&error, TeamLookupError::AliasNotFound { team_alias } if team_alias == "a1"));
    assert_http(&error, 404, Detail::Error);
    assert_eq!(world.events(), ["read team_alias:a1", "alias a1"]);
}

#[tokio::test]
async fn alias_duplicate_400() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "a1", None));
    world.lock().team_rows.push(team_row("t2", "a1", None));
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(&error, TeamLookupError::AliasAmbiguous { team_alias } if team_alias == "a1"));
    assert_http(&error, 400, Detail::Error);
    assert!(error.to_string().contains("team_id_jwt_field"));
    assert!(
        levels.seen().is_empty(),
        "the HTTP answers are raised unlogged"
    );
    assert_eq!(world.events(), ["read team_alias:a1", "alias a1"]);
}

#[tokio::test]
async fn alias_permission_debug_drop() {
    let world = world();
    world
        .lock()
        .team_rows
        .push(team_row("t1", "a1", Some("p1")));
    world.lock().failing.insert("permission".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let team = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap();
    drop(guard);
    assert_eq!(team.team.fields.object_permission, None);
    assert_eq!(levels.seen(), vec![Level::DEBUG]);
    let expected = [
        "read team_alias:a1",
        "alias a1",
        "write team_alias:a1 ttl=60",
        "write team_id:t1 ttl=60",
    ];
    assert_eq!(world.events(), expected);
}

#[tokio::test]
async fn alias_then_id_cache() {
    let world = world();
    world
        .lock()
        .team_rows
        .push(team_row("t1", "a1", Some("p1")));
    world
        .lock()
        .permission_rows
        .push(json(r#"{"object_permission_id": "p1"}"#));
    let team = get_team_object_by_alias(&lookup(&world, 9), "a1")
        .await
        .unwrap();
    assert_eq!(
        team.last_refreshed_at, None,
        "the alias path skips _cache_team_object"
    );
    assert!(team.team.fields.object_permission.is_some());
    let expected = [
        "read team_alias:a1",
        "alias a1",
        "read object_permission_id:p1",
        "permission p1 writer=false",
        "write object_permission_id:p1 ttl=60",
        "write team_alias:a1 ttl=9",
        "write team_id:t1 ttl=9",
    ];
    assert_eq!(world.events(), expected);
    let state = world.lock();
    assert_eq!(state.teams.get("team_alias:a1"), Some(&team));
    assert_eq!(state.teams.get("team_id:t1"), Some(&team));
}

#[tokio::test]
async fn alias_second_write_failure_500() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "a1", None));
    world.lock().failing.insert("write team_id:t1".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(assert_failure(&error), TeamFailure::Cache(_)));
    assert_eq!(levels.seen(), vec![Level::ERROR]);
    assert_eq!(
        world.events(),
        [
            "read team_alias:a1",
            "alias a1",
            "write team_alias:a1 ttl=60"
        ]
    );
}

#[tokio::test]
async fn alias_read_failure_500() {
    let world = world();
    world.lock().failing.insert("team".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let read = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap_err();
    assert!(matches!(assert_failure(&read), TeamFailure::Store(_)));
    world.lock().failing.clear();
    world.lock().team_rows.push(json(r#"{"team_alias": "a1"}"#));
    let invalid = get_team_object_by_alias(&lookup(&world, 60), "a1")
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(assert_failure(&invalid), TeamFailure::Row(_)));
    assert_eq!(levels.seen(), vec![Level::ERROR, Level::ERROR]);
}
