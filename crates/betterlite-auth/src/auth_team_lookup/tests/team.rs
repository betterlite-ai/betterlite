use betterlite_db::team_rows::aliases::ModelAliases;
use tracing::Level;

use super::support::{
    Levels, assert_http, cached_team, json, lookup, request, team_row, without_database, world,
};
use crate::auth_team_lookup::error::{Detail, TeamFailure, TeamLookupError};
use crate::auth_team_lookup::lookup::{TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;

fn assert_unavailable(error: &TeamLookupError) -> &TeamFailure {
    assert_http(error, 404, Detail::Error);
    let TeamLookupError::TeamUnavailable { team_id, cause } = error else {
        panic!("the ordinary team 404 is expected, got {error:?}");
    };
    assert_eq!(team_id, "t1");
    cause.as_ref()
}

#[tokio::test]
async fn db_required_before_cache() {
    let world = world();
    world
        .lock()
        .teams
        .insert("team_id:t1".to_owned(), cached_team("t1"));
    let error = get_team_object(
        &without_database(&world),
        request(TeamSource::CacheThenDatabase),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, TeamLookupError::NoDatabase));
    assert_eq!(error.status(), None);
    assert_eq!(error.detail(), None);
    assert!(
        world.events().is_empty(),
        "the cache is never read without a database"
    );
}

#[tokio::test]
async fn team_cache_hit() {
    let world = world();
    world
        .lock()
        .teams
        .insert("team_id:t1".to_owned(), cached_team("t1"));
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let team = get_team_object(&lookup(&world, 60), request(TeamSource::CacheThenDatabase))
        .await
        .unwrap();
    assert_eq!(team.team.fields.base.team_alias.as_deref(), Some("cached"));
    assert_eq!(world.events(), ["read team_id:t1"]);
}

#[tokio::test]
async fn cache_only_miss_404() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let error = get_team_object(&lookup(&world, 60), request(TeamSource::CacheOnly))
        .await
        .unwrap_err();
    assert!(matches!(&error, TeamLookupError::NotInCache { team_id } if team_id == "t1"));
    assert_http(&error, 404, Detail::Error);
    assert_eq!(world.events(), ["read team_id:t1"]);
}

#[tokio::test]
async fn writer_beats_cache_only() {
    let world = world();
    world
        .lock()
        .teams
        .insert("team_id:t1".to_owned(), cached_team("t1"));
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let team = get_team_object(&lookup(&world, 60), request(TeamSource::Writer))
        .await
        .unwrap();
    assert_eq!(team.team.fields.base.team_alias.as_deref(), Some("stored"));
    assert_eq!(team.last_refreshed_at, Some(1.5_f64));
    assert_eq!(
        world.events(),
        ["team t1 writer=true", "write team_id:t1 ttl=60"]
    );
}

#[tokio::test]
async fn grant_relations_loaded() {
    let world = world();
    let row = json(
        r#"{"team_id": "t1", "object_permission_id": "p1", "litellm_model_table": {"id": 7, "model_aliases": {"fast": "gpt-4o"}, "created_by": "a", "updated_by": "a"}}"#,
    );
    world.lock().team_rows.push(row);
    world.lock().permission_rows.push(json(
        r#"{"object_permission_id": "p1", "models": ["gpt-4o"]}"#,
    ));
    let team = get_team_object(&lookup(&world, 60), request(TeamSource::CacheThenDatabase))
        .await
        .unwrap();
    let relation = team.team.fields.litellm_model_table.clone().unwrap();
    assert!(
        matches!(&relation.model_aliases, Some(ModelAliases::Map(aliases)) if aliases.contains_key("fast"))
    );
    let permission = team.team.fields.object_permission.clone().unwrap();
    assert_eq!(permission.models.0, Some(vec!["gpt-4o".to_owned()]));
    let expected = [
        "read team_id:t1",
        "team t1 writer=false",
        "read object_permission_id:p1",
        "permission p1 writer=false",
        "write object_permission_id:p1 ttl=60",
        "write team_id:t1 ttl=60",
    ];
    assert_eq!(world.events(), expected);

    world.lock().events.clear();
    let loaded = json(
        r#"{"team_id": "t2", "object_permission_id": "p1", "object_permission": {"object_permission_id": "p1"}}"#,
    );
    world.lock().team_rows.push(loaded);
    let second = TeamRequest {
        team_id: "t2",
        ..request(TeamSource::Writer)
    };
    get_team_object(&lookup(&world, 60), second).await.unwrap();
    assert_eq!(
        world.events(),
        ["team t2 writer=true", "write team_id:t2 ttl=60"],
        "an already loaded object_permission is not read again"
    );
}

#[tokio::test]
async fn missing_team_distinct_404() {
    let world = world();
    let error = get_team_object(&lookup(&world, 60), request(TeamSource::CacheThenDatabase))
        .await
        .unwrap_err();
    assert!(matches!(&error, TeamLookupError::TeamNotFound { team_id } if team_id == "t1"));
    assert_http(&error, 404, Detail::Error);
    assert_eq!(world.events(), ["read team_id:t1", "team t1 writer=false"]);
}

#[tokio::test]
async fn throttle_miss_404() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let throttled = lookup(&world, 60);
    world.at(90);
    throttled.cadence.update("team_id:t1", None);
    world.at(149);
    let error = get_team_object(&throttled, request(TeamSource::Writer))
        .await
        .unwrap_err();
    assert!(matches!(assert_unavailable(&error), TeamFailure::Throttled));
    assert!(
        world.events().is_empty(),
        "check_db_only still waits out a recent miss"
    );
}

#[tokio::test]
async fn team_read_failure_404() {
    let world = world();
    world.lock().failing.insert("team".to_owned());
    let error = get_team_object(&lookup(&world, 60), request(TeamSource::CacheThenDatabase))
        .await
        .unwrap_err();
    assert!(matches!(assert_unavailable(&error), TeamFailure::Store(_)));
    assert_eq!(world.events(), ["read team_id:t1", "team t1 writer=false"]);
}

#[tokio::test]
async fn optional_permission_debug_drop() {
    let world = world();
    world
        .lock()
        .team_rows
        .push(team_row("t1", "stored", Some("p1")));
    world.lock().failing.insert("permission".to_owned());
    let levels = Levels::default();
    let guard = levels.install();
    let team = get_team_object(&lookup(&world, 60), request(TeamSource::CacheThenDatabase))
        .await
        .unwrap();
    drop(guard);
    assert_eq!(team.team.fields.object_permission, None);
    assert_eq!(team.team.fields.object_permission_id.as_deref(), Some("p1"));
    assert_eq!(levels.seen(), vec![Level::DEBUG]);
    assert_eq!(
        world.events(),
        [
            "read team_id:t1",
            "team t1 writer=false",
            "write team_id:t1 ttl=60"
        ]
    );
}

#[tokio::test]
async fn writer_permission_failure_404() {
    let world = world();
    world
        .lock()
        .team_rows
        .push(team_row("t1", "stored", Some("p1")));
    let levels = Levels::default();
    let guard = levels.install();
    let error = get_team_object(&lookup(&world, 60), request(TeamSource::Writer))
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(
        assert_unavailable(&error),
        TeamFailure::Permission(_)
    ));
    assert!(
        levels.seen().is_empty(),
        "the writer failure is raised, not logged"
    );
    assert_eq!(
        world.events(),
        ["team t1 writer=true", "permission p1 writer=true"]
    );
}

#[tokio::test]
async fn team_cache_before_access_time() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let cadence = lookup(&world, 60);
    world.at(10);
    cadence.cadence.update("team_id:t1", None);
    world.at(70);
    get_team_object(&cadence, request(TeamSource::Writer))
        .await
        .unwrap();
    world.at(12);
    get_team_object(&cadence, request(TeamSource::Writer))
        .await
        .unwrap();
    let expected = [
        "team t1 writer=true",
        "write team_id:t1 ttl=60",
        "team t1 writer=true",
        "write team_id:t1 ttl=60",
    ];
    assert_eq!(
        world.events(),
        expected,
        "a recorded team never throttles the next read"
    );
}

#[tokio::test]
async fn cache_failure_keeps_access_time() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "stored", None));
    world.lock().failing.insert("write team_id:t1".to_owned());
    let cadence = lookup(&world, 60);
    world.at(10);
    cadence.cadence.update("team_id:t1", None);
    world.at(70);
    let failed = get_team_object(&cadence, request(TeamSource::Writer))
        .await
        .unwrap_err();
    assert!(matches!(assert_unavailable(&failed), TeamFailure::Cache(_)));
    world.lock().failing.clear();
    world.at(12);
    let throttled = get_team_object(&cadence, request(TeamSource::Writer))
        .await
        .unwrap_err();
    assert!(matches!(
        assert_unavailable(&throttled),
        TeamFailure::Throttled
    ));
    assert_eq!(world.events(), ["team t1 writer=true"]);
}

#[tokio::test]
async fn upsert_admin_seed() {
    let world = world();
    world.lock().created = Some(team_row("t1", "created", None));
    let upsert = TeamRequest {
        team_id_upsert: true,
        ..request(TeamSource::CacheThenDatabase)
    };
    let team = get_team_object(&lookup(&world, 60), upsert).await.unwrap();
    assert_eq!(team.team.fields.base.team_alias.as_deref(), Some("created"));
    assert_eq!(team.last_refreshed_at, Some(1.5_f64));
    let expected = [
        "read team_id:t1",
        "team t1 writer=false",
        "provision t1 proxy_admin",
        "write team_id:t1 ttl=60",
    ];
    assert_eq!(world.events(), expected);
}

#[tokio::test]
async fn upsert_failure_404() {
    let world = world();
    world.lock().failing.insert("provision".to_owned());
    let upsert = TeamRequest {
        team_id_upsert: true,
        ..request(TeamSource::CacheThenDatabase)
    };
    let error = get_team_object(&lookup(&world, 60), upsert)
        .await
        .unwrap_err();
    assert!(matches!(
        assert_unavailable(&error),
        TeamFailure::Provision(_)
    ));

    let mut unprovisioned = lookup(&world, 60);
    unprovisioned.provisioner = None;
    let missing = get_team_object(&unprovisioned, upsert).await.unwrap_err();
    assert!(matches!(
        assert_unavailable(&missing),
        TeamFailure::NoProvisioner
    ));
    let expected = [
        "read team_id:t1",
        "team t1 writer=false",
        "provision t1 proxy_admin",
        "read team_id:t1",
        "team t1 writer=false",
    ];
    assert_eq!(world.events(), expected);
}

#[tokio::test]
async fn ttl_override_consumed() {
    let world = world();
    world.lock().team_rows.push(team_row("t1", "stored", None));
    let short = lookup(&world, 9);
    world.at(10);
    short.cadence.update("team_id:t1", None);
    world.at(18);
    let error = get_team_object(&short, request(TeamSource::Writer))
        .await
        .unwrap_err();
    assert!(matches!(assert_unavailable(&error), TeamFailure::Throttled));
    world.at(19);
    get_team_object(&short, request(TeamSource::Writer))
        .await
        .unwrap();
    assert_eq!(
        world.events(),
        ["team t1 writer=true", "write team_id:t1 ttl=60"]
    );
}
