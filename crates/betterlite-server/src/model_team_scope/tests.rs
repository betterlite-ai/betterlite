#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::sync_channel;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::ports::OidcHttp;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::caller_scope::{byok_row_outside_caller_teams, get_caller_byok_team_scope};
use super::error::{NOT_AUTHORIZED, ScopeDetail};
use super::filter::filter_models_by_team_id;
use super::lookup::find_model_by_id;
use super::team_models::authorize_team_id_query;
use crate::health_access_and_connections::visibility::HealthRouter;
use crate::proxy_helpers::checks::HttpException;

mod postgres;
mod support;

fn ids(rows: &[JsonObject]) -> Vec<Option<&str>> {
    rows.iter().map(support::json_id).collect()
}

fn names(rows: &[JsonObject]) -> Vec<&str> {
    rows.iter().map(support::name_of).collect()
}

async fn search_models(
    router: &dyn HealthRouter,
    reads: &support::StubReads,
    search: Option<&str>,
) -> (Vec<JsonObject>, usize) {
    let fixture = support::DecryptFixture::new();
    find_model_by_id("d1", search, Some(router), reads, &fixture.context())
        .await
        .unwrap()
}

fn team_models(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(team, models)| {
            (
                String::from(*team),
                models.iter().map(|model| String::from(*model)).collect(),
            )
        })
        .collect()
}

#[tokio::test]
async fn unscoped_admin_missing_context() {
    let reads = support::StubReads::default();
    assert_eq!(get_caller_byok_team_scope(None, Some(&reads)).await, None);
    assert_eq!(
        get_caller_byok_team_scope(Some(&support::admin_caller()), Some(&reads)).await,
        None
    );
    let alice = support::caller(Some("alice"), Some("team-a"));
    assert_eq!(
        get_caller_byok_team_scope(Some(&alice), None::<&support::StubReads>).await,
        None
    );
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn key_scope_no_user() {
    let reads = support::StubReads::default();
    let keyed = support::caller(None, Some("team-a"));
    assert_eq!(
        get_caller_byok_team_scope(Some(&keyed), Some(&reads)).await,
        Some(support::set(&["team-a"]))
    );
    let bare = support::caller(None, None);
    assert_eq!(
        get_caller_byok_team_scope(Some(&bare), Some(&reads)).await,
        Some(support::set(&[]))
    );
    let empty_team = support::caller(None, Some(""));
    assert_eq!(
        get_caller_byok_team_scope(Some(&empty_team), Some(&reads)).await,
        Some(support::set(&[]))
    );
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn live_user_scope_union() {
    let reads = support::StubReads::default().with_users(BTreeMap::from([(
        String::from("alice"),
        vec![String::from("team-a"), String::from("team-key")],
    )]));
    let alice = support::caller(Some("alice"), Some("team-key"));
    assert_eq!(
        get_caller_byok_team_scope(Some(&alice), Some(&reads)).await,
        Some(support::set(&["team-a", "team-key"]))
    );
    assert_eq!(reads.calls(), vec![String::from("user:alice")]);
}

#[tokio::test]
async fn lookup_failure_key_scope_log() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default().with_failing(vec![support::Failing::User]);
    let alice = support::caller(Some("alice"), Some("team-key"));
    assert_eq!(
        get_caller_byok_team_scope(Some(&alice), Some(&reads)).await,
        Some(support::set(&["team-key"]))
    );
    let text = logs.contents();
    assert!(text.contains("ERROR"));
    assert!(text.contains("Failed to look up caller teams while scoping BYOK search"));
}

#[test]
fn byok_other_team_hidden() {
    let info = support::object(&[("team_id", support::text("team-b"))]);
    assert!(byok_row_outside_caller_teams(
        &info,
        Some(&support::set(&["team-a"]))
    ));
    assert!(!byok_row_outside_caller_teams(
        &info,
        Some(&support::set(&["team-b"]))
    ));
}

#[test]
fn nonteam_rows_unrestricted() {
    let info = support::object(&[("team_id", Json::Null)]);
    assert!(!byok_row_outside_caller_teams(
        &info,
        Some(&support::set(&[]))
    ));
    let absent = support::object(&[]);
    assert!(!byok_row_outside_caller_teams(
        &absent,
        Some(&support::set(&["team-a"]))
    ));
    assert!(!byok_row_outside_caller_teams(&info, None));
}

#[tokio::test]
async fn empty_scope_denies_byok() {
    let reads = support::StubReads::default();
    let alice = support::caller(Some("alice"), None);
    let scope = get_caller_byok_team_scope(Some(&alice), Some(&reads))
        .await
        .expect("a caller without an admin view is scoped");
    assert!(scope.is_empty());
    let owned = support::object(&[("team_id", support::text("team-a"))]);
    assert!(byok_row_outside_caller_teams(&owned, Some(&scope)));
    let global = support::object(&[("id", support::text("g1"))]);
    assert!(!byok_row_outside_caller_teams(&global, Some(&scope)));
}

#[tokio::test]
async fn explicit_team_requires_user_membership() {
    let reads = support::StubReads::default().with_users(BTreeMap::from([(
        String::from("alice"),
        vec![String::from("team-a")],
    )]));
    let admin = support::admin_caller();
    authorize_team_id_query("team-other", &admin, &reads)
        .await
        .unwrap();
    let alice = support::caller(Some("alice"), None);
    authorize_team_id_query("team-a", &alice, &reads)
        .await
        .unwrap();
    let error = authorize_team_id_query("team-other", &alice, &reads)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        error.detail(),
        ScopeDetail::Error {
            error: NOT_AUTHORIZED
        }
    );
    let anonymous = support::caller(None, Some("team-a"));
    let denied = authorize_team_id_query("team-a", &anonymous, &reads)
        .await
        .unwrap_err();
    assert_eq!(denied.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        reads.calls(),
        vec![String::from("user:alice"), String::from("user:alice")]
    );
}

#[tokio::test]
async fn team_lookup_failure_403() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default().with_failing(vec![support::Failing::User]);
    let alice = support::caller(Some("alice"), None);
    let error = authorize_team_id_query("team-a", &alice, &reads)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        error.detail(),
        ScopeDetail::Error {
            error: NOT_AUTHORIZED
        }
    );
    let text = logs.contents();
    assert!(text.contains("ERROR"));
    assert!(text.contains("Failed to look up caller teams while authorizing teamId filter"));
}

#[tokio::test]
async fn authorize_before_team_read() {
    let reads = support::StubReads::default().with_users(BTreeMap::from([(
        String::from("alice"),
        vec![String::from("team-a")],
    )]));
    let alice = support::caller(Some("alice"), None);
    let models = vec![support::deployment_json(
        "gpt-4o",
        &[("id", support::text("d1"))],
    )];
    let error = filter_models_by_team_id(&models, "team-other", &reads, None, Some(&alice))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(reads.calls(), vec![String::from("user:alice")]);
}

#[tokio::test]
async fn missing_team_warn_empty() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default();
    let models = vec![support::deployment_json(
        "gpt-4o",
        &[("id", support::text("d1"))],
    )];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, None, None)
        .await
        .unwrap();
    assert!(kept.is_empty());
    let text = logs.contents();
    assert!(text.contains("WARN"));
    assert!(text.contains("Team team-a not found in database"));
}

#[tokio::test]
async fn team_read_error_empty() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default().with_failing(vec![support::Failing::Team]);
    let models = vec![support::deployment_json(
        "gpt-4o",
        &[("id", support::text("d1"))],
    )];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, None, None)
        .await
        .unwrap();
    assert!(kept.is_empty());
    let text = logs.contents();
    assert!(text.contains("ERROR"));
    assert!(text.contains("Error fetching team team-a:"));
}

#[tokio::test]
async fn all_models_excludes_foreign_byok() {
    let specs = [
        support::spec("gpt-4o", &[("id", support::text("g1"))]),
        support::spec(
            "team-gpt",
            &[
                ("id", support::text("t1")),
                ("team_id", support::text("team-a")),
            ],
        ),
        support::spec(
            "foreign-gpt",
            &[
                ("id", support::text("t2")),
                ("team_id", support::text("team-b")),
            ],
        ),
    ];
    let router = support::FixtureRouter::new(&specs);
    // A stored row named like the team's entry: the all-proxy branch never asks the database for it.
    let reads = support::StubReads::default()
        .with_teams(team_models(&[("team-a", &["all-proxy-models"])]))
        .with_rows(support::stored_rows(&[("g2", "all-proxy-models")]));
    let mut models = support::dumped(&specs);
    models.push(support::deployment_json(
        "hidden-gpt",
        &[("id", support::text("g2"))],
    ));
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("g1"), Some("t1")]);
}

#[tokio::test]
async fn named_access_group_expansion() {
    let grouped = support::listed(&["group-a"]);
    let router = support::FixtureRouter::new(&[
        support::spec(
            "group-model",
            &[
                ("id", support::text("d1")),
                ("access_groups", grouped.clone()),
            ],
        ),
        support::spec(
            "second-group-model",
            &[("id", support::text("d2")), ("access_groups", grouped)],
        ),
        support::spec("unrelated", &[("id", support::text("d3"))]),
    ]);
    let reads = support::StubReads::default().with_teams(team_models(&[("team-a", &["group-a"])]));
    let models = vec![
        support::deployment_json("group-model", &[("id", support::text("d1"))]),
        support::deployment_json("second-group-model", &[("id", support::text("d2"))]),
        support::deployment_json("unrelated", &[("id", support::text("d3"))]),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("d1"), Some("d2")]);
    let expected = vec![vec![
        String::from("group-model"),
        String::from("second-group-model"),
    ]];
    assert_eq!(reads.names_seen(), expected);
}

// Two catalog rows that share one name, and the stored rows the name query answers.
fn db_names_fixture() -> (support::FixtureRouter, support::StubReads) {
    let grouped = support::listed(&["group-a"]);
    let specs = [
        support::spec(
            "c",
            &[
                ("id", support::text("r1")),
                ("access_groups", grouped.clone()),
            ],
        ),
        support::spec(
            "c",
            &[("id", support::text("r2")), ("access_groups", grouped)],
        ),
    ];
    let stored = [("d3", "b"), ("d1", "c"), ("d4", "zzz")];
    let reads = support::StubReads::default()
        .with_teams(team_models(&[("team-a", &["b", "group-a"])]))
        .with_rows(support::stored_rows(&stored));
    (support::FixtureRouter::new(&specs), reads)
}

#[tokio::test]
async fn db_names_only() {
    let (router, reads) = db_names_fixture();
    let models = vec![
        support::deployment_json("c", &[("id", support::text("r1"))]),
        support::deployment_json("c", &[("id", support::text("d1"))]),
        support::deployment_json("c", &[("id", support::text("r2"))]),
        support::deployment_json("b", &[("id", support::text("d3"))]),
        support::deployment_json("zzz", &[("id", support::text("d4"))]),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(
        ids(&kept),
        vec![Some("r1"), Some("d1"), Some("r2"), Some("d3")]
    );
    let expected = vec![vec![
        String::from("b"),
        String::from("c"),
        String::from("c"),
    ]];
    assert_eq!(reads.names_seen(), expected);
}

#[tokio::test]
async fn db_query_error_keeps_router_ids() {
    let (logs, _guard) = support::capture_logs();
    let router =
        support::FixtureRouter::new(&[support::spec("gpt-4o", &[("id", support::text("r1"))])]);
    let reads = support::StubReads::default()
        .with_teams(team_models(&[("team-a", &["gpt-4o"])]))
        .with_failing(vec![support::Failing::Names]);
    let models = vec![
        support::deployment_json("gpt-4o", &[("id", support::text("r1"))]),
        support::deployment_json("stored-only", &[("id", support::text("d2"))]),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("r1")]);
    let text = logs.contents();
    assert!(text.contains("DEBUG"));
    assert!(text.contains("Error querying database models for team team-a:"));
}

#[tokio::test]
async fn missing_ids_omitted() {
    let router = support::FixtureRouter::new(&[
        support::spec("gpt-4o", &[("id", support::text("g1"))]),
        support::unnumbered("no-id-gpt", &[]),
    ]);
    let reads =
        support::StubReads::default().with_teams(team_models(&[("team-a", &["all-proxy-models"])]));
    let models = vec![
        support::deployment_json("gpt-4o", &[("id", support::text("g1"))]),
        support::deployment_json("no-id-gpt", &[]),
        support::deployment_json("empty-id-gpt", &[("id", support::text(""))]),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("g1")]);
}

#[tokio::test]
async fn access_via_list_only() {
    let reads = support::StubReads::default().with_teams(team_models(&[("team-a", &[])]));
    let models = vec![
        support::deployment_json(
            "annotated",
            &[
                ("id", support::text("a1")),
                ("team_id", support::text("team-b")),
                ("access_via_team_ids", support::listed(&["team-a"])),
            ],
        ),
        support::deployment_json(
            "text-annotation",
            &[
                ("id", support::text("b1")),
                ("access_via_team_ids", support::text("team-a")),
            ],
        ),
        support::deployment_json(
            "other-annotation",
            &[
                ("id", support::text("c1")),
                ("access_via_team_ids", support::listed(&["team-b"])),
            ],
        ),
        support::deployment_json(
            "empty-annotation",
            &[
                ("id", support::text("d1")),
                ("access_via_team_ids", support::listed(&[])),
            ],
        ),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, None, None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("a1")]);
}

#[tokio::test]
async fn empty_models_unrestricted() {
    let router =
        support::FixtureRouter::new(&[support::spec("gpt-4o", &[("id", support::text("g1"))])]);
    let reads = support::StubReads::default().with_teams(team_models(&[("team-a", &[])]));
    let models = vec![
        support::deployment_json("gpt-4o", &[("id", support::text("g1"))]),
        support::deployment_json(
            "foreign-gpt",
            &[
                ("id", support::text("t2")),
                ("team_id", support::text("team-b")),
            ],
        ),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("g1")]);
}

#[tokio::test]
async fn direct_access_does_not_widen() {
    let router =
        support::FixtureRouter::new(&[support::spec("team-model", &[("id", support::text("t1"))])]);
    let reads =
        support::StubReads::default().with_teams(team_models(&[("team-a", &["team-model"])]));
    let models = vec![
        support::deployment_json(
            "byok-direct",
            &[
                ("id", support::text("d9")),
                ("direct_access", Json::Bool(true)),
            ],
        ),
        support::deployment_json("team-model", &[("id", support::text("t1"))]),
        support::deployment_json(
            "own-model",
            &[
                ("id", support::text("t2")),
                ("team_id", support::text("team-a")),
                ("direct_access", Json::Bool(true)),
            ],
        ),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("t1"), Some("t2")]);
}

#[tokio::test]
async fn stable_filter_order() {
    let router = support::FixtureRouter::new(&[
        support::spec("gpt-4o", &[("id", support::text("g1"))]),
        support::spec(
            "team-gpt",
            &[
                ("id", support::text("o1")),
                ("team_id", support::text("team-a")),
            ],
        ),
    ]);
    let reads =
        support::StubReads::default().with_teams(team_models(&[("team-a", &["all-proxy-models"])]));
    let owned = support::deployment_json(
        "team-gpt",
        &[
            ("id", support::text("o1")),
            ("team_id", support::text("team-a")),
        ],
    );
    let models = vec![
        owned.clone(),
        support::deployment_json("gpt-4o", &[("id", support::text("g1"))]),
        owned,
        support::deployment_json(
            "foreign-gpt",
            &[
                ("id", support::text("f1")),
                ("team_id", support::text("team-b")),
            ],
        ),
    ];
    let kept = filter_models_by_team_id(&models, "team-a", &reads, Some(&router), None)
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("o1"), Some("g1"), Some("o1")]);
}

#[tokio::test]
async fn config_before_db_deep_copy() {
    let router =
        support::FixtureRouter::new(&[support::spec("gpt-4o", &[("id", support::text("d1"))])]);
    let reads = support::StubReads::default().with_rows(vec![support::model_row(
        r#"{"model_id":"d1","model_name":"db-name","litellm_params":{"model":"openai/gpt-4o"}}"#,
    )]);
    let fixture = support::DecryptFixture::new();
    let (models, count) = find_model_by_id("d1", None, Some(&router), &reads, &fixture.context())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(names(&models), vec!["gpt-4o"]);
    assert!(reads.calls().is_empty());
    let mut answer = models;
    answer[0].insert(String::from("model_name"), support::text("changed"));
    let (again, _) = find_model_by_id("d1", None, Some(&router), &reads, &fixture.context())
        .await
        .unwrap();
    assert_eq!(names(&again), vec!["gpt-4o"]);
}

#[tokio::test]
async fn db_decrypted_first_row() {
    let reads = support::StubReads::default().with_rows(vec![support::model_row(
        r#"{"model_id":"d1","model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o","api_key":"sk-plain"},"model_info":{"id":"stored","team_id":"team-a"}}"#,
    )]);
    let fixture = support::DecryptFixture::new();
    let (models, count) = find_model_by_id("d1", None, None, &reads, &fixture.context())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(names(&models), vec!["gpt-4o"]);
    assert_eq!(support::json_id(&models[0]), Some("d1"));
    let Some(Json::Object(params)) = models[0].get("litellm_params") else {
        panic!("a decrypted deployment keeps its params");
    };
    assert_eq!(params.get("api_key"), Some(&support::text("sk-plain")));
    assert_eq!(reads.calls(), vec![String::from("row:d1")]);
}

#[tokio::test]
async fn decryption_error_logs_empty() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default().with_rows(vec![support::model_row(
        r#"{"model_id":"d1","model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o","api_key":"os.environ/oidc/google"}}"#,
    )]);
    let fixture = support::DecryptFixture::new();
    let (models, count) = find_model_by_id("d1", None, None, &reads, &fixture.context())
        .await
        .unwrap();
    assert!(models.is_empty());
    assert_eq!(count, 0);
    let text = logs.contents();
    assert!(text.contains("ERROR"));
    assert!(text.contains("Error querying database for modelId d1"));
}

#[tokio::test]
async fn a_blocked_secret_read_leaves_the_async_worker_free() {
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = sync_channel(1);
    let http = Arc::new(support::BlockedOidc::new(started, release_rx));
    let reads = support::StubReads::default().with_rows(vec![support::model_row(
        r#"{"model_id":"d1","model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o","api_key":"os.environ/oidc/google/audience"}}"#,
    )]);
    let concrete = Arc::clone(&http);
    let oidc: Arc<dyn OidcHttp> = concrete;
    let fixture = support::DecryptFixture::with_oidc_http(oidc);
    let context = fixture.context();
    let mut lookups = JoinSet::new();
    lookups.spawn(async move { find_model_by_id("d1", None, None, &reads, &context).await });
    timeout(support::OIDC_WAIT, started_rx)
        .await
        .expect("the secret read starts")
        .expect("the OIDC fake announces its read");
    let _released = release.try_send(());
    let (models, count) = lookups
        .join_next()
        .await
        .expect("the lookup task runs")
        .expect("the lookup task joins")
        .expect("the lookup answers");
    assert!(models.is_empty());
    assert_eq!(count, 0);
    assert!(
        http.released_while_waiting(),
        "an async task ran while the secret read blocked"
    );
}

#[tokio::test]
async fn cancelling_lookup_keeps_decryption_tracked_until_shutdown_drain() {
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = sync_channel(1);
    let http = Arc::new(support::BlockedOidc::new(started, release_rx));
    let concrete = Arc::clone(&http);
    let oidc: Arc<dyn OidcHttp> = concrete;
    let fixture = support::DecryptFixture::with_oidc_http(oidc);
    let tasks = TaskTracker::new();
    let context = fixture.context_with_tasks(tasks.clone());
    let reads = support::StubReads::default().with_rows(vec![support::model_row(
        r#"{"model_id":"d1","model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o","api_key":"os.environ/oidc/google/audience"}}"#,
    )]);
    let mut lookups = JoinSet::new();
    lookups.spawn(async move { find_model_by_id("d1", None, None, &reads, &context).await });
    timeout(support::OIDC_WAIT, started_rx)
        .await
        .expect("the secret read starts")
        .expect("the OIDC fake announces its read");

    lookups.abort_all();
    let lookup_result = lookups.join_next().await.expect("lookup task joins");
    let Err(cancelled_error) = lookup_result else {
        panic!("the request lookup was cancelled");
    };
    assert!(cancelled_error.is_cancelled());
    assert!(!tasks.is_empty());

    tasks.close();
    let shutdown_tracker = tasks.clone();
    let (drain_started, drain_started_rx) = oneshot::channel();
    let mut drainers = JoinSet::new();
    drainers.spawn(async move {
        let _announced = drain_started.send(());
        shutdown_tracker.wait().await;
    });
    drain_started_rx.await.expect("the shutdown drain starts");
    if let Some(completed) = drainers.try_join_next() {
        panic!("the shutdown drain finished before decryption: {completed:?}");
    }

    let _released = release.try_send(());
    drainers
        .join_next()
        .await
        .expect("the shutdown drain runs")
        .expect("the shutdown drain completes");
    assert!(tasks.is_empty());
    assert!(http.released_while_waiting());
}

#[tokio::test]
async fn trimmed_search() {
    let router =
        support::FixtureRouter::new(&[support::spec("gpt-4o", &[("id", support::text("d1"))])]);
    let reads = support::StubReads::default();
    let (models, count) = search_models(&router, &reads, None).await;
    assert_eq!(count, 1);
    assert_eq!(names(&models), vec!["gpt-4o"]);
    let (blank, blank_count) = search_models(&router, &reads, Some("")).await;
    assert_eq!(blank_count, 1);
    assert_eq!(names(&blank), vec!["gpt-4o"]);
    let (spaces, spaces_count) = search_models(&router, &reads, Some("   ")).await;
    assert_eq!(spaces_count, 1);
    assert_eq!(names(&spaces), vec!["gpt-4o"]);
    let (padded, padded_count) = search_models(&router, &reads, Some("  GPT-4O  ")).await;
    assert_eq!(padded_count, 1);
    assert_eq!(names(&padded), vec!["gpt-4o"]);
    let (missing, missing_count) = search_models(&router, &reads, Some("mini")).await;
    assert!(missing.is_empty());
    assert_eq!(missing_count, 0);
}

#[tokio::test]
async fn stored_row_read_error_logs_empty() {
    let (logs, _guard) = support::capture_logs();
    let reads = support::StubReads::default().with_failing(vec![support::Failing::Row]);
    let fixture = support::DecryptFixture::new();
    let (models, count) = find_model_by_id("d1", None, None, &reads, &fixture.context())
        .await
        .unwrap();
    assert!(models.is_empty());
    assert_eq!(count, 0);
    let text = logs.contents();
    assert!(text.contains("ERROR"));
    assert!(text.contains("Error querying database for modelId d1"));
}

#[tokio::test]
async fn unknown_deployment() {
    let reads = support::StubReads::default();
    let fixture = support::DecryptFixture::new();
    let (models, count) = find_model_by_id("missing", None, None, &reads, &fixture.context())
        .await
        .unwrap();
    assert!(models.is_empty());
    assert_eq!(count, 0);
    assert_eq!(reads.calls(), vec![String::from("row:missing")]);
}

#[tokio::test]
async fn router_lookup_failure_is_500() {
    let reads = support::StubReads::default().with_teams(team_models(&[("team-a", &["gpt-4o"])]));
    let models = vec![support::deployment_json(
        "gpt-4o",
        &[("id", support::text("d1"))],
    )];
    let error = filter_models_by_team_id(
        &models,
        "team-a",
        &reads,
        Some(&support::FailingRouter),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    let failure = HttpException::from(error);
    assert_eq!(failure.status, StatusCode::INTERNAL_SERVER_ERROR);
}
