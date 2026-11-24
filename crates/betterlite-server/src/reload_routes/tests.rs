#![cfg(test)]

mod support;

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::reload_schedule::ReloadSchedule;
use betterlite_models::source::{CatalogProvenance, SourceKind};
use chrono::{DateTime, Utc};
use rstest::rstest;

use crate::auth::auth_identity::AuthContext;
use crate::periodic_reloads::state::PeriodicReloads;

use self::support::{
    BETA_RELOAD, BETA_SCHEDULE, BETA_SCHEDULE_2_HOURS, BETA_STATUS, COST_MAP_RELOAD,
    COST_MAP_SCHEDULE, COST_MAP_SCHEDULE_2_HOURS, COST_MAP_SOURCE, COST_MAP_STATUS, FORBIDDEN_ROLE,
    FakeCatalog, FakeStore, Harness, READS, STORE_FAILURE, WRITES, at, at_year, context,
    env_forced_provenance, every_route_answer, json_data, key_caller, key_with, local_provenance,
    master, moment, one_hour_before_datetime_max, parsed, platform_admin, platform_viewer,
    provenance_loaded_at, reloaded_provenance, send, snapshot, with_micros,
};

#[tokio::test]
async fn oneshot_rejects_an_anonymous_caller() {
    let harness = Harness::new();
    let (status, text) = send(&harness, None, Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(parsed(&text)["error"]["code"], "401");
    let (beta_code, _) = send(&harness, None, Method::GET, BETA_STATUS).await;
    assert_eq!(beta_code, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oneshot_rejects_a_non_admin_on_every_write_route() {
    let harness = Harness::new();
    every_route_answer(
        &harness,
        &key_with("internal_user"),
        &WRITES,
        Some("internal_user"),
    )
    .await;
    let state = harness.store().snapshot();
    assert!(state.written_intervals.is_empty());
    assert!(state.beta_writes.is_empty());
    assert_eq!(state.beta_deletes, 0);
    assert_eq!(state.cleared, 0);
    assert_eq!(harness.beta.reloads(), 0);
}

#[rstest]
#[case::admin_viewer(key_with("proxy_admin_viewer"), "proxy_admin_viewer")]
#[case::platform_viewer(platform_viewer(), "proxy_admin_viewer")]
#[case::org_admin(key_with("org_admin"), "org_admin")]
#[case::internal_user(key_with("internal_user_viewer"), "internal_user_viewer")]
#[case::no_role(key_caller("{}"), "internal_user")]
#[case::platform_org_admin(
    context("jwt-user", &[Role::OrgAdmin], SourceKey(None)),
    "org_admin"
)]
#[tokio::test]
async fn oneshot_renders_the_current_role_in_the_write_denial(
    #[case] who: AuthContext,
    #[case] role: &str,
) {
    let (status, text) = send(&Harness::new(), Some(who), Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(parsed(&text)["detail"], format!("{FORBIDDEN_ROLE} {role}"));
}

#[rstest]
#[case::master(master())]
#[case::proxy_admin_key(key_with("proxy_admin"))]
#[case::platform_admin(platform_admin())]
#[tokio::test]
async fn oneshot_admits_an_admin_to_every_write_route(#[case] who: AuthContext) {
    let harness = Harness::new();
    every_route_answer(&harness, &who, &WRITES, None).await;
}

#[tokio::test]
async fn oneshot_rejects_a_caller_without_admin_view_on_every_read_route() {
    let harness = Harness::new();
    every_route_answer(
        &harness,
        &key_with("internal_user"),
        &READS,
        Some("internal_user"),
    )
    .await;
}

// a failing store would answer 500, so the 403 proves the role check ran before the read on every read route
#[tokio::test]
async fn oneshot_rejects_a_caller_without_admin_view_before_the_store_read() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::failing());
    every_route_answer(
        &harness,
        &key_with("internal_user"),
        &READS,
        Some("internal_user"),
    )
    .await;
}

#[rstest]
#[case::master(master())]
#[case::proxy_admin_key(key_with("proxy_admin"))]
#[case::admin_viewer_key(key_with("proxy_admin_viewer"))]
#[case::platform_admin(platform_admin())]
#[case::platform_viewer(platform_viewer())]
#[tokio::test]
async fn oneshot_admits_every_admin_view_caller_to_the_read_routes(#[case] who: AuthContext) {
    let harness = Harness::new();
    every_route_answer(&harness, &who, &READS, None).await;
}

#[rstest]
#[case::cost_map(Method::POST, "/schedule/model_cost_map_reload?hours=0")]
#[case::beta(Method::POST, "/schedule/anthropic_beta_headers_reload?hours=0")]
#[tokio::test]
async fn oneshot_rejects_a_non_admin_before_the_hours_check(
    #[case] method: Method,
    #[case] uri: &str,
) {
    let (status, text) = send(
        &Harness::new(),
        Some(key_with("internal_user")),
        method,
        uri,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        parsed(&text)["detail"],
        format!("{FORBIDDEN_ROLE} internal_user")
    );
}

#[tokio::test]
async fn oneshot_rejects_a_non_admin_before_the_database_check() {
    let mut harness = Harness::new();
    harness.store = None;
    let caller = key_with("internal_user");
    every_route_answer(&harness, &caller, &WRITES, Some("internal_user")).await;
    every_route_answer(&harness, &caller, &READS, Some("internal_user")).await;
    assert_eq!(harness.catalog.reloads(), 0);
    assert_eq!(harness.beta.reloads(), 0);
}

#[rstest]
#[case::cost_map(COST_MAP_SCHEDULE)]
#[case::beta(BETA_SCHEDULE)]
#[tokio::test]
async fn oneshot_answers_422_when_hours_is_missing(#[case] uri: &str) {
    let expected = parsed(
        r#"{"detail": [{"type": "missing", "loc": ["query", "hours"], "msg": "Field required"}]}"#,
    );
    let (status, text) = send(&Harness::new(), Some(master()), Method::POST, uri).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parsed(&text), expected);
}

#[rstest]
#[case::text("abc")]
#[case::float("1.5")]
#[case::empty("")]
#[case::boolean("true")]
#[case::overflow_low("-99999999999999999999")]
#[case::overflow_high("99999999999999999999")]
#[tokio::test]
async fn oneshot_answers_422_when_hours_is_not_an_integer(#[case] value: &str) {
    let expected = parsed(
        r#"{"detail": [{"type": "int_parsing", "loc": ["query", "hours"], "msg": "Input should be a valid integer, unable to parse string as an integer"}]}"#,
    );
    let uri = format!("{COST_MAP_SCHEDULE}?hours={value}");
    let (status, text) = send(&Harness::new(), Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parsed(&text), expected);
}

#[rstest]
#[case::cost_map(COST_MAP_SCHEDULE)]
#[case::beta(BETA_SCHEDULE)]
#[tokio::test]
async fn oneshot_answers_422_before_the_role_check(#[case] uri: &str) {
    let caller = key_with("internal_user");
    let (status, _) = send(&Harness::new(), Some(caller), Method::POST, uri).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
}

#[rstest]
#[case::cost_map_zero(COST_MAP_SCHEDULE, "0")]
#[case::cost_map_minus_one(COST_MAP_SCHEDULE, "-1")]
#[case::cost_map_minimum(COST_MAP_SCHEDULE, "-9223372036854775808")]
#[case::beta_zero(BETA_SCHEDULE, "0")]
#[case::beta_minus_one(BETA_SCHEDULE, "-1")]
#[case::beta_minimum(BETA_SCHEDULE, "-9223372036854775808")]
#[tokio::test]
async fn oneshot_rejects_a_non_positive_interval(#[case] schedule: &str, #[case] value: &str) {
    let harness = Harness::new();
    let uri = format!("{schedule}?hours={value}");
    let (status, text) = send(&harness, Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    assert_eq!(
        parsed(&text),
        parsed(r#"{"detail": "Hours must be greater than 0"}"#)
    );
    assert!(harness.store().snapshot().written_intervals.is_empty());
    assert!(harness.store().snapshot().beta_writes.is_empty());
}

#[tokio::test]
async fn oneshot_schedules_the_cost_map_reload() {
    let harness = Harness::new();
    let uri = format!("{COST_MAP_SCHEDULE}?hours=24");
    let (status, text) = send(&harness, Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"message": "Model cost map reload scheduled for every 24 hours", "status": "success", "interval_hours": 24, "timestamp": "2026-11-23T12:00:00+00:00"}"#
        )
    );
    assert_eq!(harness.store().snapshot().written_intervals, vec![24]);
}

#[tokio::test]
async fn oneshot_rejects_a_cost_map_interval_the_schedule_cannot_store() {
    let uri = format!("{COST_MAP_SCHEDULE}?hours=4294967296");
    let (status, text) = send(&Harness::new(), Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        parsed(&text)["detail"],
        "Failed to schedule model cost map reload: the interval 4294967296 hours does not fit the stored schedule"
    );
}

#[tokio::test]
async fn oneshot_cancels_the_cost_map_reload() {
    let harness = Harness::new();
    let (status, text) = send(&harness, Some(master()), Method::DELETE, COST_MAP_SCHEDULE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"message": "Model cost map reload schedule cancelled", "status": "success", "timestamp": "2026-11-23T12:00:00+00:00"}"#
        )
    );
    assert_eq!(harness.store().snapshot().cleared, 1);
}

#[rstest]
#[case::cost_map_schedule(
    Method::POST,
    COST_MAP_SCHEDULE_2_HOURS,
    "Failed to schedule model cost map reload"
)]
#[case::cost_map_cancel(
    Method::DELETE,
    COST_MAP_SCHEDULE,
    "Failed to cancel model cost map reload"
)]
#[case::beta_schedule(
    Method::POST,
    BETA_SCHEDULE_2_HOURS,
    "Failed to schedule anthropic beta headers reload"
)]
#[case::beta_cancel(
    Method::DELETE,
    BETA_SCHEDULE,
    "Failed to cancel anthropic beta headers reload"
)]
#[case::beta_reload(Method::POST, BETA_RELOAD, "Failed to reload anthropic beta headers")]
#[tokio::test]
async fn oneshot_wraps_the_missing_database(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] prefix: &str,
) {
    let mut harness = Harness::new();
    harness.store = None;
    let (status, text) = send(&harness, Some(master()), method, uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}");
    assert_eq!(
        parsed(&text)["detail"],
        format!("{prefix}: 500: Database connection not available")
    );
}

#[rstest]
#[case::cost_map_reload(Method::POST, COST_MAP_RELOAD, "Failed to reload model cost map")]
#[case::cost_map_schedule(
    Method::POST,
    COST_MAP_SCHEDULE_2_HOURS,
    "Failed to schedule model cost map reload"
)]
#[case::cost_map_cancel(
    Method::DELETE,
    COST_MAP_SCHEDULE,
    "Failed to cancel model cost map reload"
)]
#[case::cost_map_status(
    Method::GET,
    COST_MAP_STATUS,
    "Failed to get model cost map reload status"
)]
#[case::beta_reload(Method::POST, BETA_RELOAD, "Failed to reload anthropic beta headers")]
#[case::beta_schedule(
    Method::POST,
    BETA_SCHEDULE_2_HOURS,
    "Failed to schedule anthropic beta headers reload"
)]
#[case::beta_cancel(
    Method::DELETE,
    BETA_SCHEDULE,
    "Failed to cancel anthropic beta headers reload"
)]
#[case::beta_status(
    Method::GET,
    BETA_STATUS,
    "Failed to get anthropic beta headers reload status"
)]
#[tokio::test]
async fn oneshot_wraps_a_store_failure(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] prefix: &str,
) {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::failing());
    let (status, text) = send(&harness, Some(master()), method, uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}");
    assert_eq!(
        parsed(&text)["detail"],
        format!("{prefix}: {STORE_FAILURE}")
    );
}

#[tokio::test]
async fn oneshot_reloads_the_cost_map_and_adopts_the_revision() {
    let previous = at(10, 0, 0);
    let mut harness = Harness::new();
    harness.reloads = Arc::new(PeriodicReloads::new(
        &provenance_loaded_at(previous),
        previous,
    ));
    harness.store = Some(FakeStore::with_revision(7));
    let (status, text) = send(&harness, Some(master()), Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"message": "Price data reloaded successfully! 3 models updated.", "status": "success", "models_count": 3, "timestamp": "2026-11-23T12:00:00+00:00", "source_revision": "rev-1", "etag": "etag-1"}"#
        )
    );
    assert_eq!(harness.catalog.reloads(), 1);
    assert_eq!(harness.reloads.cost_map_loaded_at(), moment());
    assert_eq!(harness.reloads.cost_map_applied_revision(), 7);
    assert_eq!(harness.store().snapshot().recorded, vec![moment()]);
}

#[tokio::test]
async fn oneshot_keeps_the_map_when_the_reload_is_unavailable() {
    let previous = at(10, 0, 0);
    let mut harness = Harness::new();
    harness.reloads = Arc::new(PeriodicReloads::new(
        &provenance_loaded_at(previous),
        previous,
    ));
    harness.catalog = FakeCatalog::unavailable("HTTP 503 from https://example.test/cost.json");
    let (status, text) = send(&harness, Some(master()), Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"detail": "Failed to reload model cost map: HTTP 503 from https://example.test/cost.json. Current pricing data was kept."}"#
        )
    );
    assert_eq!(harness.reloads.cost_map_loaded_at(), previous);
    assert_eq!(harness.reloads.cost_map_applied_revision(), 0);
    assert!(harness.store().snapshot().recorded.is_empty());
    assert_eq!(harness.catalog.adopted().catalog.adopted_size, 4);
}

#[tokio::test]
async fn oneshot_keeps_a_failed_recorded_revision_from_being_applied() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::failing());
    let (status, _) = send(&harness, Some(master()), Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(harness.reloads.cost_map_loaded_at(), moment());
    assert_eq!(harness.reloads.cost_map_applied_revision(), 0);
}

#[tokio::test]
async fn oneshot_keeps_the_missing_database_unwrapped_for_the_cost_map_reload() {
    let mut harness = Harness::new();
    harness.store = None;
    let (status, text) = send(&harness, Some(master()), Method::POST, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        parsed(&text),
        parsed(r#"{"detail": "Database connection not available"}"#)
    );
    assert_eq!(harness.catalog.reloads(), 0);
}

#[tokio::test]
async fn oneshot_answers_the_cost_map_status_without_a_database() {
    let mut harness = Harness::new();
    harness.store = None;
    harness.catalog = FakeCatalog::with_current(snapshot(reloaded_provenance(), 3));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": false, "interval_hours": null, "last_run": null, "next_run": null, "source_revision": "rev-1", "etag": "etag-1"}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_the_cost_map_status_from_the_stored_schedule() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_schedule(ReloadSchedule {
        interval_hours: Some(6),
        reload_revision: 2,
        last_run_at: Some(at(6, 0, 0)),
    }));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": true, "interval_hours": 6, "last_run": "2026-11-23T06:00:00+00:00", "next_run": "2026-11-23T12:00:00+00:00", "source_revision": "rev-1", "etag": "etag-1"}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_a_cost_map_status_for_a_schedule_without_an_interval() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_schedule(ReloadSchedule {
        interval_hours: None,
        reload_revision: 2,
        last_run_at: Some(at(6, 0, 0)),
    }));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    let reply = parsed(&text);
    assert_eq!(reply["scheduled"], false);
    assert_eq!(reply["interval_hours"], JsonValue::Null);
    assert_eq!(reply["last_run"], "2026-11-23T06:00:00+00:00");
    assert_eq!(reply["next_run"], JsonValue::Null);
}

#[rstest]
#[case::local(local_provenance(), "local", false)]
#[case::remote(reloaded_provenance(), "remote", false)]
#[case::env_forced(env_forced_provenance(), "local", true)]
#[tokio::test]
async fn oneshot_answers_the_cost_map_source(
    #[case] provenance: CatalogProvenance,
    #[case] source: &str,
    #[case] env_forced: bool,
) {
    let mut harness = Harness::new();
    harness.catalog = FakeCatalog::with_current(snapshot(provenance, 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_SOURCE).await;
    assert_eq!(status, StatusCode::OK);
    let reply = parsed(&text);
    assert_eq!(reply["source"], source);
    assert_eq!(reply["is_env_forced"], env_forced);
    assert_eq!(reply["model_count"], 0_i32);
}

#[tokio::test]
async fn oneshot_answers_every_cost_map_source_field() {
    let mut harness = Harness::new();
    let provenance = CatalogProvenance {
        kind: SourceKind::Local,
        url: Some("https://example.test/cost.json".to_owned()),
        is_env_forced: false,
        fallback_reason: Some("HTTP 503 from https://example.test/cost.json".to_owned()),
        loaded_at: Some(at(9, 0, 0)),
        source_revision: Some("rev-9".to_owned()),
        etag: None,
    };
    harness.catalog = FakeCatalog::with_current(snapshot(provenance, 12));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_SOURCE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"source": "local", "url": "https://example.test/cost.json", "is_env_forced": false, "fallback_reason": "HTTP 503 from https://example.test/cost.json", "loaded_at": "2026-11-23T09:00:00+00:00", "source_revision": "rev-9", "etag": null, "model_count": 12}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_every_remote_cost_map_source_field() {
    let mut harness = Harness::new();
    harness.catalog = FakeCatalog::with_current(snapshot(reloaded_provenance(), 3));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_SOURCE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"source": "remote", "url": "https://example.test/cost.json", "is_env_forced": false, "fallback_reason": null, "loaded_at": "2026-11-23T12:00:00+00:00", "source_revision": "rev-1", "etag": "etag-1", "model_count": 3}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_an_empty_local_cost_map_source() {
    let mut harness = Harness::new();
    harness.catalog = FakeCatalog::with_current(snapshot(local_provenance(), 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_SOURCE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"source": "local", "url": null, "is_env_forced": false, "fallback_reason": null, "loaded_at": null, "source_revision": null, "etag": null, "model_count": 0}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_the_cost_map_source_without_a_database() {
    let mut harness = Harness::new();
    harness.store = None;
    let (status, text) = send(&harness, Some(master()), Method::GET, COST_MAP_SOURCE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&text)["model_count"], 3_i32);
}

#[tokio::test]
async fn oneshot_reloads_the_beta_headers_and_preserves_the_interval() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 6, "force_reload": false}"#,
    )));
    let (status, text) = send(&harness, Some(master()), Method::POST, BETA_RELOAD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"message": "Anthropic beta headers configuration reloaded successfully! 2 providers updated.", "status": "success", "providers_count": 2, "timestamp": "2026-11-23T12:00:00"}"#
        )
    );
    assert_eq!(harness.beta.reloads(), 1);
    assert_eq!(harness.reloads.last_beta_headers_reload(), Some(moment()));
    assert_eq!(
        harness.store().snapshot().beta_writes,
        vec![json_data(r#"{"interval_hours": 6, "force_reload": true}"#)]
    );
    assert_eq!(harness.store().snapshot().beta_invalidations, 1);
}

#[rstest]
#[case::no_row(FakeStore::default())]
#[case::null_interval(FakeStore::with_beta(json_data(
    r#"{"interval_hours": null, "force_reload": true}"#
)))]
#[tokio::test]
async fn oneshot_reloads_the_beta_headers_without_a_stored_interval(#[case] store: FakeStore) {
    let mut harness = Harness::new();
    harness.store = Some(store);
    let (status, _) = send(&harness, Some(master()), Method::POST, BETA_RELOAD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.store().snapshot().beta_writes,
        vec![json_data(
            r#"{"interval_hours": null, "force_reload": true}"#
        )]
    );
    assert_eq!(harness.store().snapshot().beta_invalidations, 1);
}

#[tokio::test]
async fn oneshot_schedules_then_cancels_the_beta_reload() {
    let harness = Harness::new();
    let uri = format!("{BETA_SCHEDULE}?hours=12");
    let (status, text) = send(&harness, Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"message": "Anthropic beta headers reload scheduled for every 12 hours", "status": "success", "interval_hours": 12, "timestamp": "2026-11-23T12:00:00"}"#
        )
    );
    assert_eq!(
        harness.store().snapshot().beta_writes,
        vec![json_data(
            r#"{"interval_hours": 12, "force_reload": false}"#
        )]
    );
    let (cancel_status, cancel_text) =
        send(&harness, Some(master()), Method::DELETE, BETA_SCHEDULE).await;
    assert_eq!(cancel_status, StatusCode::OK);
    assert_eq!(
        parsed(&cancel_text),
        parsed(
            r#"{"message": "Anthropic beta headers reload schedule cancelled", "status": "success", "timestamp": "2026-11-23T12:00:00"}"#
        )
    );
    assert_eq!(harness.store().snapshot().beta_deletes, 1);
    assert_eq!(harness.store().snapshot().beta_invalidations, 2);
}

#[tokio::test]
async fn oneshot_writes_microseconds_like_python_for_the_beta_timestamps() {
    let mut harness = Harness::new();
    harness.now = with_micros(moment(), 123_456);
    let uri = format!("{BETA_SCHEDULE}?hours=2");
    let (status, text) = send(&harness, Some(master()), Method::POST, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&text)["timestamp"], "2026-11-23T12:00:00.123456");
}

#[tokio::test]
async fn oneshot_answers_the_beta_status_without_a_database() {
    let mut harness = Harness::new();
    harness.store = None;
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": false, "interval_hours": null, "last_run": null, "next_run": null}"#
        )
    );
}

#[rstest]
#[case::no_row(FakeStore::default())]
#[case::null_interval(FakeStore::with_beta(json_data(
    r#"{"interval_hours": null, "force_reload": false}"#
)))]
#[case::not_a_number(FakeStore::with_beta(json_data(
    r#"{"interval_hours": "6", "force_reload": false}"#
)))]
#[case::not_an_object(FakeStore::with_beta(JsonData::String("6".to_owned())))]
#[tokio::test]
async fn oneshot_answers_a_beta_status_without_an_interval(#[case] store: FakeStore) {
    let mut harness = Harness::new();
    harness.store = Some(store);
    harness.reloads.set_last_beta_headers_reload(moment());
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": false, "interval_hours": null, "last_run": null, "next_run": null}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_the_beta_status_before_the_next_run() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 2, "force_reload": false}"#,
    )));
    harness.reloads.set_last_beta_headers_reload(at(11, 0, 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": true, "interval_hours": 2, "last_run": "2026-11-23T11:00:00", "next_run": "2026-11-23T13:00:00"}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_a_beta_status_after_the_interval_passed() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 2, "force_reload": false}"#,
    )));
    harness.reloads.set_last_beta_headers_reload(at(9, 0, 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": true, "interval_hours": 2, "last_run": "2026-11-23T09:00:00", "next_run": null}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_a_beta_status_at_the_exact_interval() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 2, "force_reload": false}"#,
    )));
    harness.reloads.set_last_beta_headers_reload(at(10, 0, 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&text)["next_run"], JsonValue::Null);
}

#[tokio::test]
async fn oneshot_answers_a_beta_status_for_a_last_reload_in_the_future() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 2, "force_reload": false}"#,
    )));
    harness.reloads.set_last_beta_headers_reload(at(13, 0, 0));
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": true, "interval_hours": 2, "last_run": "2026-11-23T13:00:00", "next_run": "2026-11-23T15:00:00"}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_a_beta_status_the_pod_never_reloaded() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 2, "force_reload": false}"#,
    )));
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(r#"{"scheduled": true, "interval_hours": 2, "last_run": null, "next_run": null}"#)
    );
}

#[rstest]
#[case::python_datemax(
    json_data(r#"{"interval_hours": 48, "force_reload": false}"#),
    at_year(9999, 12, 31, 23)
)]
#[case::timedelta_overflow(
    json_data(r#"{"interval_hours": 9223372036854775807, "force_reload": false}"#),
    at(13, 0, 0)
)]
#[tokio::test]
async fn oneshot_answers_a_beta_status_whose_next_run_overflows(
    #[case] stored: JsonData,
    #[case] last_reload: DateTime<Utc>,
) {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(stored));
    harness.reloads.set_last_beta_headers_reload(last_reload);
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    let reply = parsed(&text);
    assert_eq!(reply["scheduled"], true);
    assert_eq!(reply["next_run"], JsonValue::Null);
}

#[tokio::test]
async fn oneshot_answers_a_beta_status_whose_next_run_is_datetime_max() {
    let mut harness = Harness::new();
    harness.store = Some(FakeStore::with_beta(json_data(
        r#"{"interval_hours": 1, "force_reload": false}"#,
    )));
    harness
        .reloads
        .set_last_beta_headers_reload(one_hour_before_datetime_max());
    let (status, text) = send(&harness, Some(master()), Method::GET, BETA_STATUS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&text),
        parsed(
            r#"{"scheduled": true, "interval_hours": 1, "last_run": "9999-12-31T22:59:59.999999", "next_run": "9999-12-31T23:59:59.999999"}"#
        )
    );
}

#[tokio::test]
async fn oneshot_answers_405_when_a_route_takes_another_method() {
    let harness = Harness::new();
    let (status, _) = send(&harness, Some(master()), Method::GET, COST_MAP_RELOAD).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (source_status, _) = send(&harness, Some(master()), Method::POST, COST_MAP_SOURCE).await;
    assert_eq!(source_status, StatusCode::METHOD_NOT_ALLOWED);
}
