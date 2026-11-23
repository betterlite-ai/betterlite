use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::table_ops::rows::ConfigParamRow;
use chrono::{DateTime, Utc};
use rstest::rstest;

use super::support::{
    Harness, beta_row, beta_row_without_force_reload, capture_logs, moment, moment_at,
    raw_beta_row, run_beta_check,
};

#[tokio::test]
async fn an_absent_row_does_nothing() {
    let harness = Harness::new(moment(1));
    run_beta_check(&harness).await;
    assert_eq!(harness.beta.calls(), 0);
    assert!(harness.rows.writes().is_empty());
    assert_eq!(harness.rows.evictions(), 0);
}

#[rstest]
#[case::a_null_value(raw_beta_row(JsonData::Null))]
#[case::no_interval_and_no_force(beta_row(None, false))]
#[tokio::test]
async fn a_config_with_nothing_to_do_does_nothing(#[case] row: ConfigParamRow) {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.set_beta_row(Some(row));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 0);
    assert!(harness.rows.writes().is_empty());
    assert_eq!(harness.rows.evictions(), 0);
    // a null value is no configuration at all, not a value of the wrong type
    assert!(
        !logs
            .contents()
            .contains("Error in _check_and_reload_anthropic_beta_headers")
    );
}

#[rstest]
#[case::without_an_interval(None, JsonData::Null)]
#[case::with_an_interval(Some(5), JsonData::Number(serde_json::Number::from(5_u32)))]
#[tokio::test]
async fn a_force_reload_reloads_and_clears_the_flag(
    #[case] interval_hours: Option<u32>,
    #[case] stored_interval: JsonData,
) {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness
        .rows
        .set_beta_row(Some(beta_row(interval_hours, true)));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 1);
    assert_eq!(
        harness.checks.reloads.last_beta_headers_reload(),
        Some(moment(1))
    );
    let stored = JsonData::Object(BTreeMap::from([
        ("force_reload".to_owned(), JsonData::Bool(false)),
        ("interval_hours".to_owned(), stored_interval),
    ]));
    assert_eq!(harness.rows.writes(), vec![stored]);
    assert_eq!(harness.rows.evictions(), 1);
    let text = logs.contents();
    assert!(text.contains("Anthropic beta headers reload triggered by force reload flag"));
    assert!(text.contains("Anthropic beta headers config reloaded successfully. Providers: 3"));
}

#[tokio::test]
async fn an_interval_without_a_recorded_reload_reloads() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.set_beta_row(Some(beta_row(Some(2), false)));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 1);
    assert!(
        logs.contents()
            .contains("Anthropic beta headers reload triggered - no previous reload time recorded")
    );
}

#[tokio::test]
async fn an_absent_force_flag_reads_as_false_and_the_interval_reloads() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness
        .rows
        .set_beta_row(Some(beta_row_without_force_reload(2)));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 1);
    let text = logs.contents();
    assert!(
        text.contains("Anthropic beta headers reload triggered - no previous reload time recorded")
    );
    assert!(!text.contains("Anthropic beta headers reload triggered by force reload flag"));
}

#[rstest]
#[case::just_before_the_interval(moment_at(2, 59), false, None)]
#[case::exactly_at_the_interval(
    moment(3),
    true,
    Some("Hours since last reload: 2.00, Interval: 2")
)]
#[case::half_an_hour_past_it(
    moment_at(3, 30),
    true,
    Some("Hours since last reload: 2.50, Interval: 2")
)]
#[tokio::test]
async fn an_interval_reload_measures_from_the_last_reload(
    #[case] now: DateTime<Utc>,
    #[case] due: bool,
    #[case] expected_log: Option<&str>,
) {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(now);
    harness.rows.set_beta_row(Some(beta_row(Some(2), false)));
    harness
        .checks
        .reloads
        .set_last_beta_headers_reload(moment(1));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), usize::from(due));
    let text = logs.contents();
    if due {
        let expected = expected_log.expect("the due case names its log line");
        assert!(text.contains(expected));
        assert_eq!(harness.checks.reloads.last_beta_headers_reload(), Some(now));
    } else {
        assert!(!text.contains("Anthropic beta headers reload triggered by interval"));
        assert_eq!(
            harness.checks.reloads.last_beta_headers_reload(),
            Some(moment(1))
        );
    }
}

#[tokio::test]
async fn a_config_with_a_wrong_typed_interval_logs_and_does_not_reload() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    let wrong = JsonData::Object(BTreeMap::from([(
        "interval_hours".to_owned(),
        JsonData::String("two".to_owned()),
    )]));
    harness.rows.set_beta_row(Some(raw_beta_row(wrong)));
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 0);
    let text = logs.contents();
    assert!(text.contains(
        "Error in _check_and_reload_anthropic_beta_headers: the anthropic beta headers reload config has a field of the wrong type: "
    ));
    assert!(text.contains("interval_hours"));
}

#[tokio::test]
async fn a_read_failure_is_logged_and_swallowed() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.fail("read_beta_headers");
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 0);
    assert!(logs.contents().contains(
        "Error in _check_and_reload_anthropic_beta_headers: the anthropic beta headers reload state could not be read or written: the fake config repository refused the read_beta_headers call"
    ));
}

#[tokio::test]
async fn a_failed_write_logs_and_keeps_the_last_reload_time() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.set_beta_row(Some(beta_row(None, true)));
    harness.rows.fail("write_beta_headers");
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.beta.calls(), 1);
    assert_eq!(
        harness.checks.reloads.last_beta_headers_reload(),
        Some(moment(1))
    );
    assert_eq!(harness.rows.evictions(), 0);
    assert!(logs.contents().contains(
        "Error in _check_and_reload_anthropic_beta_headers: the anthropic beta headers reload state could not be read or written: the fake config repository refused the write_beta_headers call"
    ));
}

#[tokio::test]
async fn a_failed_eviction_logs_after_the_write() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.set_beta_row(Some(beta_row(None, true)));
    harness.rows.fail("evict_beta_headers");
    run_beta_check(&harness).await;
    drop(guard);
    assert_eq!(harness.rows.writes().len(), 1);
    assert!(logs.contents().contains(
        "Error in _check_and_reload_anthropic_beta_headers: the anthropic beta headers reload state could not be read or written: the fake config repository refused the evict_beta_headers call"
    ));
}
