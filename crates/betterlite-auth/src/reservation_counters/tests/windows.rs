use std::time::{Duration, UNIX_EPOCH};

use rust_decimal::Decimal;

use super::support::{Fixture, Logs, auth, dec, decoded_row, epoch_seconds, only};

#[tokio::test]
async fn window_parse_reads_each_budget_limit_duration() {
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5},"#,
        r#"{"budget_duration":"1h","max_budget":"6.5"},"#,
        r#"{"budget_duration":"1d","max_budget":0},"#,
        r#"{"budget_duration":"1d","max_budget":true},"#,
        r#"{"budget_duration":"1d","max_budget":null},"#,
        r#"{"budget_duration":"","max_budget":7}]}"#,
    ));
    let list = fixture.counters().await;
    let durations: Vec<Option<&str>> = list
        .iter()
        .map(|counter| counter.window_duration.as_deref())
        .collect();
    assert_eq!(durations, vec![Some("1d"), Some("1h"), Some("1d")]);
    let caps: Vec<Decimal> = list.iter().map(|counter| counter.max_budget).collect();
    assert_eq!(caps, vec![dec("5"), dec("6.5"), dec("1")]);
    assert_eq!(fixture.durations.specs(), vec!["1d", "1h", "1d"]);
}

#[tokio::test]
async fn window_parse_skips_a_non_string_duration() {
    let (logs, _guard) = Logs::capture();
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":42,"max_budget":5},"#,
        r#"{"budget_duration":"1d","max_budget":5}]}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.window_duration.as_deref(), Some("1d"));
    assert_eq!(fixture.durations.specs(), vec!["1d"]);
    let text = logs.text();
    assert!(text.contains("not a string"), "{text}");
    assert!(!text.contains("invalid duration"), "{text}");
}

#[tokio::test]
async fn window_start_subtracts_the_duration_from_the_reset_or_now() {
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5,"reset_at":"2024-01-03T00:00:00Z"},"#,
        r#"{"budget_duration":"1h","max_budget":5}]}"#,
    ));
    let list = fixture.counters().await;
    let starts: Vec<i64> = list
        .iter()
        .map(|counter| epoch_seconds(counter.window_start.expect("the window start")))
        .collect();
    assert_eq!(starts, vec![1_704_153_600, 1_704_063_600]);
}

#[tokio::test]
async fn window_utc_treats_a_naive_reset_at_as_utc() {
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5,"reset_at":"2024-01-03T00:00:00"}]}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(
        epoch_seconds(counter.window_start.expect("the window start")),
        1_704_153_600
    );
}

#[tokio::test]
async fn window_start_accepts_a_reset_before_the_epoch() {
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5,"reset_at":"1969-12-30T00:00:00Z"}]}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    let expected = UNIX_EPOCH
        .checked_sub(Duration::from_secs(259_200))
        .expect("the pre-epoch start");
    assert_eq!(counter.window_start, Some(expected));
}

#[tokio::test]
async fn window_start_accepts_a_year_one_reset() {
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5,"reset_at":"0001-01-01T00:00:00Z"}]}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    let expected = UNIX_EPOCH
        .checked_sub(Duration::from_secs(62_135_683_200))
        .expect("the year-one start");
    assert_eq!(counter.window_start, Some(expected));
}

#[tokio::test]
async fn window_skip_invalid_duration_warns_and_stays_out() {
    let (logs, _guard) = Logs::capture();
    let mut fixture = Fixture::new();
    fixture.token = auth(concat!(
        r#"{"token":"tok","budget_limits":["#,
        r#"{"budget_duration":"1w","max_budget":5},"#,
        r#"{"budget_duration":"1d","max_budget":5}]}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.window_duration.as_deref(), Some("1d"));
    let text = logs.text();
    assert!(
        text.contains("Skipping budget window with invalid duration"),
        "{text}"
    );
    assert!(text.contains("budget_duration=1w"), "{text}");
    assert!(!text.contains("tok"), "{text}");
}

#[tokio::test]
async fn team_window_limits_become_counters() {
    let mut fixture = Fixture::new();
    fixture.team = Some(decoded_row(concat!(
        r#"{"team_id":"team-a","spend":3,"budget_limits":["#,
        r#"{"budget_duration":"1d","max_budget":5,"reset_at":"2024-01-03T00:00:00Z"}]}"#,
    )));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:team:team-a:window:1d");
    assert_eq!(counter.entity_id, "team-a:1d");
    assert_eq!(counter.spend_log_entity_id.as_deref(), Some("team-a"));
    assert_eq!(counter.window_duration.as_deref(), Some("1d"));
    assert_eq!(
        epoch_seconds(counter.window_start.expect("the window start")),
        1_704_153_600
    );
    assert!(counter.source_cache_key.is_none());
    assert_eq!(counter.max_budget, dec("5"));
    assert_eq!(counter.fallback_spend, dec("0"));
}

#[tokio::test]
async fn window_metadata_names_the_entity_and_duration() {
    let mut fixture = Fixture::new();
    fixture.token =
        auth(r#"{"token":"tok","budget_limits":[{"budget_duration":"1d","max_budget":5}]}"#);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:key:tok:window:1d");
    assert_eq!(counter.entity_id, "tok:1d");
    assert_eq!(counter.spend_log_entity_id.as_deref(), Some("tok"));
    assert_eq!(counter.window_duration.as_deref(), Some("1d"));
    assert_eq!(
        epoch_seconds(counter.window_start.expect("the window start")),
        1_703_980_800
    );
    assert!(counter.source_cache_key.is_none());
}
