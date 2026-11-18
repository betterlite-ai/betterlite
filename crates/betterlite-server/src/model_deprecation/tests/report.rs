use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::model_deprecation::report::{classify, collect_from_specs};
use crate::model_deprecation::types::{
    DEFAULT_CHECK_INTERVAL_SECONDS, DEFAULT_DEPRECATION_WARN_DAYS, DEPRECATION_IDLE_POLL_SECONDS,
    DeprecationStatus, ModelDeprecationInfo, ModelDeprecationResponse,
};

use super::support::{
    FixedClock, TestCatalog, calendar_date, capture_debug_logs, moment, names, object, spec, text,
};

fn dated(name: &str, model: &str, date: &str) -> DeploymentSpec {
    let info = object(vec![
        ("deprecation_date", text(date)),
        ("litellm_provider", text("override")),
    ]);
    spec(name, model, info)
}

fn fixed_clock() -> FixedClock {
    FixedClock(moment("2026-05-01T12:00:00Z"))
}

#[test]
fn already_deprecated() {
    let specs = vec![
        dated("thirty-days-ago", "raw/one", "2026-04-01"),
        dated("yesterday", "raw/two", "2026-04-30"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(
        names(&response.deprecated),
        ["thirty-days-ago", "yesterday"]
    );
    assert_eq!(response.deprecated[0].days_until_deprecation, -30);
    assert_eq!(response.deprecated[1].days_until_deprecation, -1);
    assert_eq!(response.deprecated[0].status, DeprecationStatus::Deprecated);
    assert!(response.imminent.is_empty());
    assert!(response.upcoming.is_empty());
}

#[test]
fn invalid_date() {
    let specs = vec![
        dated("bad-day", "raw/a", "2026-02-30"),
        dated("bad-text", "raw/b", "not a date"),
        dated("dated", "raw/c", "2026-05-10"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(names(&response.imminent), ["dated"]);
    assert!(response.deprecated.is_empty());
    assert!(response.upcoming.is_empty());
}

#[test]
fn upcoming_boundary() {
    let specs = vec![
        dated("at-window", "raw/a", "2026-05-31"),
        dated("past-window", "raw/b", "2026-06-01"),
        dated("today", "raw/c", "2026-05-01"),
        dated("yesterday", "raw/d", "2026-04-30"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(names(&response.imminent), ["today", "at-window"]);
    assert_eq!(names(&response.upcoming), ["past-window"]);
    assert_eq!(names(&response.deprecated), ["yesterday"]);
    assert_eq!(response.imminent[0].days_until_deprecation, 0);
    assert_eq!(response.imminent[1].days_until_deprecation, 30);
    assert_eq!(response.upcoming[0].days_until_deprecation, 31);
}

#[test]
fn deduplicated_model() {
    let first = object(vec![
        ("deprecation_date", text("2026-05-10")),
        ("litellm_provider", text("first")),
    ]);
    let second = object(vec![
        ("deprecation_date", text("2026-05-10")),
        ("litellm_provider", text("second")),
    ]);
    let later = object(vec![
        ("deprecation_date", text("2026-05-20")),
        ("litellm_provider", text("later")),
    ]);
    let specs = vec![
        spec("alpha", "raw/first", first),
        spec("alpha", "raw/second", second),
        spec("alpha", "raw/later", later),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(response.imminent.len(), 2);
    assert_eq!(
        response.imminent[0].litellm_model.as_deref(),
        Some("raw/first")
    );
    assert_eq!(
        response.imminent[0].litellm_provider.as_deref(),
        Some("first")
    );
    assert_eq!(
        response.imminent[1].deprecation_date,
        calendar_date("2026-05-20")
    );
}

#[test]
fn same_name_two_dates() {
    let specs = vec![
        dated("alpha", "raw/one", "2026-05-20"),
        dated("alpha", "raw/two", "2026-05-10"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(names(&response.imminent), ["alpha", "alpha"]);
    assert_eq!(
        response.imminent[0].deprecation_date,
        calendar_date("2026-05-10")
    );
    assert_eq!(
        response.imminent[1].deprecation_date,
        calendar_date("2026-05-20")
    );
}

#[test]
fn bucket_tie_order() {
    let specs = vec![
        dated("zulu", "raw/z", "2026-05-10"),
        dated("alpha", "raw/a", "2026-05-10"),
        dated("mike", "raw/m", "2026-05-10"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(names(&response.imminent), ["alpha", "mike", "zulu"]);
}

#[test]
fn no_deployments() {
    let response = collect_from_specs(
        &[],
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert!(response.deprecated.is_empty());
    assert!(response.imminent.is_empty());
    assert!(response.upcoming.is_empty());
    assert_eq!(response.warn_within_days, DEFAULT_DEPRECATION_WARN_DAYS);
    assert_eq!(response.checked_at, moment("2026-05-01T12:00:00Z"));
}

#[test]
fn explicit_today_clock() {
    let specs = vec![dated("alpha", "raw/a", "2026-05-20")];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        Some(calendar_date("2026-06-01")),
    );
    assert_eq!(response.checked_at, moment("2026-05-01T12:00:00Z"));
    assert_eq!(response.deprecated[0].days_until_deprecation, -12);
    assert_eq!(response.deprecated[0].status, DeprecationStatus::Deprecated);

    let implicit = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(implicit.imminent[0].days_until_deprecation, 19);
}

#[test]
fn today_from_clock() {
    let specs = vec![dated("alpha", "raw/a", "2026-05-01")];
    let clock = FixedClock(moment("2026-05-01T23:59:59Z"));
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &clock,
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(response.imminent[0].days_until_deprecation, 0);
    assert_eq!(response.checked_at, moment("2026-05-01T23:59:59Z"));
}

#[test]
fn negative_window() {
    let specs = vec![
        dated("today", "raw/a", "2026-05-01"),
        dated("yesterday", "raw/b", "2026-04-30"),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        -1,
        None,
    );
    assert_eq!(response.warn_within_days, -1);
    assert_eq!(names(&response.upcoming), ["today"]);
    assert_eq!(names(&response.deprecated), ["yesterday"]);
    assert!(response.imminent.is_empty());
}

#[test]
fn missing_public_name() {
    let info = object(vec![("deprecation_date", text("2026-05-10"))]);
    let specs = vec![
        spec("alpha", "raw/a", info.clone()),
        spec("", "raw/b", info),
    ];
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    assert_eq!(names(&response.imminent), ["alpha"]);
}

#[test]
fn debug_dedup_count() {
    let specs = vec![
        dated("alpha", "raw/a", "2026-05-10"),
        dated("alpha", "raw/b", "2026-05-10"),
        dated("beta", "raw/c", "2026-05-20"),
        spec("gamma", "raw/d", JsonObject::new()),
        spec("delta", "raw/e", JsonObject::new()),
    ];
    let (captured, guard) = capture_debug_logs();
    let response = collect_from_specs(
        &specs,
        &TestCatalog::new(Vec::new()),
        &fixed_clock(),
        DEFAULT_DEPRECATION_WARN_DAYS,
        None,
    );
    drop(guard);
    assert_eq!(names(&response.imminent), ["alpha", "beta"]);
    let contents = captured.contents();
    assert!(
        contents.contains("model_deprecation: 2/5 deployments carry a deprecation date"),
        "captured {contents:?}"
    );
}

#[test]
fn response_defaults() {
    let response: ModelDeprecationResponse =
        serde_json::from_str(r#"{"warn_within_days":30,"checked_at":"2026-05-01T12:00:00Z"}"#)
            .expect("the minimal snapshot decodes");
    assert!(response.deprecated.is_empty());
    assert!(response.imminent.is_empty());
    assert!(response.upcoming.is_empty());

    let info: ModelDeprecationInfo = serde_json::from_str(
        r#"{"model_name":"alpha","deprecation_date":"2026-05-10","days_until_deprecation":9,"status":"imminent"}"#,
    )
    .expect("the minimal model decodes");
    assert_eq!(info.litellm_model, None);
    assert_eq!(info.litellm_provider, None);
    assert_eq!(info.status, DeprecationStatus::Imminent);
    assert_eq!(
        serde_json::to_string(&info).expect("the model encodes"),
        r#"{"model_name":"alpha","litellm_model":null,"deprecation_date":"2026-05-10","days_until_deprecation":9,"status":"imminent","litellm_provider":null}"#
    );
    assert!(
        serde_json::from_str::<ModelDeprecationInfo>(
            r#"{"model_name":"alpha","deprecation_date":"2026-05-10","days_until_deprecation":9,"status":"later"}"#,
        )
        .is_err(),
        "an unknown status is rejected"
    );
}

#[test]
fn poll_constants() {
    assert_eq!(DEFAULT_DEPRECATION_WARN_DAYS, 30);
    assert_eq!(DEFAULT_CHECK_INTERVAL_SECONDS, 86_400);
    assert_eq!(DEPRECATION_IDLE_POLL_SECONDS, 30);
}

#[test]
fn classify_uses_the_inclusive_window() {
    assert_eq!(classify(-1, 30), DeprecationStatus::Deprecated);
    assert_eq!(classify(0, 30), DeprecationStatus::Imminent);
    assert_eq!(classify(30, 30), DeprecationStatus::Imminent);
    assert_eq!(classify(31, 30), DeprecationStatus::Upcoming);
}
